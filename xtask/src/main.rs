//! 开发工具，不随产品发布。
//!
//! `cargo xtask nib-dump [Navicat.app] [输出目录]`：把 Navicat 编译后的 nib 导出为 JSON 视图树。
//! `cargo xtask help-index [Navicat.app] [输出文件]`：整理 Navicat 自带帮助页的文件名与标题。
//! `cargo xtask nib-tree <nib 名称>`：以紧凑树形打印已导出的 nib（类名、frame、文案、字体），写规格时用。

use std::{collections::HashSet, fs, path::PathBuf};

use nibarchive::{NIBArchive, ValueVariant};
use serde_json::{Map, Value as Json, json};

const DEFAULT_APP: &str = "/Applications/Navicat Premium.app";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let app = PathBuf::from(args.get(1).map(String::as_str).unwrap_or(DEFAULT_APP));
    let result = match args.first().map(String::as_str) {
        Some("nib-dump") => nib_dump(&app, args.get(2).map(PathBuf::from)),
        Some("help-index") => help_index(&app, args.get(2).map(PathBuf::from)),
        Some("nib-tree") if args.len() > 1 => nib_tree(&args[1]),
        _ => Err(
            "用法：cargo xtask <nib-dump|help-index> [Navicat.app] [输出路径] | nib-tree <名称>"
                .into(),
        ),
    };
    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

type Result<T> = std::result::Result<T, String>;

fn nib_dump(app: &std::path::Path, out: Option<PathBuf>) -> Result<()> {
    let src = app.join("Contents/Resources/Base.lproj");
    let out = out.unwrap_or_else(|| PathBuf::from("reference/nib"));
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut entries: Vec<_> = fs::read_dir(&src)
        .map_err(|e| format!("{}: {e}", src.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "nib"))
        .collect();
    entries.sort();
    let (mut ok, mut failed) = (0, 0);
    for path in entries {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        match NIBArchive::from_file(&path) {
            Ok(archive) => {
                let doc = Nib::new(&archive).dump(&name);
                let text = serde_json::to_string_pretty(&doc).unwrap();
                fs::write(out.join(format!("{name}.json")), text).map_err(|e| e.to_string())?;
                ok += 1;
            }
            Err(err) => {
                eprintln!("跳过 {name}: {err:?}");
                failed += 1;
            }
        }
    }
    println!("导出 {ok} 个 nib 到 {}，失败 {failed} 个", out.display());
    Ok(())
}

/// 不在视图属性里展开的键：父子/响应链等反向引用会造成环或噪音。
const SKIP_KEYS: &[&str] = &[
    "NSSubviews",
    "NSSuperview",
    "NSNextResponder",
    "NSNextKeyView",
    "NSWindow",
    "NSControlView",
    "NSTarget",
    "NSDelegate",
    "NSDataSource",
];

struct Nib<'a> {
    archive: &'a NIBArchive,
}

impl<'a> Nib<'a> {
    fn new(archive: &'a NIBArchive) -> Self {
        Self { archive }
    }

    fn class(&self, ix: usize) -> &str {
        self.archive.objects()[ix]
            .class_name(self.archive.class_names())
            .name()
    }

    fn fields(&self, ix: usize) -> Vec<(&str, &ValueVariant)> {
        let obj = &self.archive.objects()[ix];
        obj.values(self.archive.values())
            .iter()
            .map(|v| (v.key(self.archive.keys()).as_str(), v.value()))
            .collect()
    }

    fn field(&self, ix: usize, key: &str) -> Option<&ValueVariant> {
        self.fields(ix)
            .into_iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }

    /// 字符串类对象（NSString 及其变体）直接还原为文本。
    fn as_string(&self, ix: usize) -> Option<String> {
        if !self.class(ix).contains("String") {
            return None;
        }
        match self.field(ix, "NS.bytes") {
            Some(ValueVariant::Data(bytes)) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        }
    }

    fn array_items(&self, ix: usize) -> Vec<usize> {
        self.fields(ix)
            .into_iter()
            .filter_map(|(k, v)| match (k, v) {
                ("UINibEncoderEmptyKey" | "NS.objects", ValueVariant::ObjectRef(r)) => {
                    Some(*r as usize)
                }
                _ => None,
            })
            .collect()
    }

    fn primitive(&self, v: &ValueVariant) -> Json {
        match v {
            ValueVariant::Int8(n) => json!(n),
            ValueVariant::Int16(n) => json!(n),
            ValueVariant::Int32(n) => json!(n),
            ValueVariant::Int64(n) => json!(n),
            ValueVariant::Bool(b) => json!(b),
            ValueVariant::Float(f) => json!(f),
            ValueVariant::Double(f) => json!(f),
            ValueVariant::Data(bytes) => match std::str::from_utf8(bytes) {
                Ok(s) if s.chars().all(|c| !c.is_control() || c == '\0') => {
                    json!(s.trim_end_matches('\0'))
                }
                _ => json!(format!("<{} bytes>", bytes.len())),
            },
            ValueVariant::Nil => Json::Null,
            ValueVariant::ObjectRef(r) => json!(format!("#{r}")),
        }
    }

    /// 按深度展开对象引用，视图本身不在属性里展开（由视图树负责）。
    fn resolve(&self, ix: usize, depth: usize, seen: &mut HashSet<usize>) -> Json {
        if let Some(s) = self.as_string(ix) {
            return json!(s);
        }
        let class = self.class(ix);
        if class.ends_with("Array") || class.ends_with("Set") {
            if depth == 0 {
                return json!(format!("{class}#{ix}"));
            }
            let items = self.array_items(ix);
            return Json::Array(
                items
                    .into_iter()
                    .map(|i| self.resolve(i, depth - 1, seen))
                    .collect(),
            );
        }
        if depth == 0 || !seen.insert(ix) || self.is_view(ix) {
            return json!(format!("{class}#{ix}"));
        }
        let mut map = Map::new();
        map.insert("class".into(), json!(class));
        for (k, v) in self.fields(ix) {
            if SKIP_KEYS.contains(&k) {
                continue;
            }
            let value = match v {
                ValueVariant::ObjectRef(r) => self.resolve(*r as usize, depth - 1, seen),
                other => self.primitive(other),
            };
            map.insert(k.to_string(), value);
        }
        seen.remove(&ix);
        Json::Object(map)
    }

    fn is_view(&self, ix: usize) -> bool {
        self.fields(ix)
            .iter()
            .any(|(k, _)| matches!(*k, "NSFrame" | "NSFrameSize" | "NSvFlags"))
    }

    fn subviews(&self, ix: usize) -> Vec<usize> {
        match self.field(ix, "NSSubviews") {
            Some(ValueVariant::ObjectRef(r)) => self.array_items(*r as usize),
            _ => Vec::new(),
        }
    }

    fn view_tree(&self, ix: usize, seen: &mut HashSet<usize>) -> Json {
        let mut node = Map::new();
        node.insert("id".into(), json!(ix));
        node.insert("class".into(), json!(self.class(ix)));
        let mut attrs = Map::new();
        for (k, v) in self.fields(ix) {
            if SKIP_KEYS.contains(&k) {
                continue;
            }
            let value = match v {
                ValueVariant::ObjectRef(r) => self.resolve(*r as usize, 3, seen),
                other => self.primitive(other),
            };
            attrs.insert(k.to_string(), value);
        }
        node.insert("attrs".into(), Json::Object(attrs));
        let children: Vec<Json> = self
            .subviews(ix)
            .into_iter()
            .map(|c| self.view_tree(c, seen))
            .collect();
        if !children.is_empty() {
            node.insert("subviews".into(), Json::Array(children));
        }
        Json::Object(node)
    }

    fn dump(&self, name: &str) -> Json {
        let count = self.archive.objects().len();
        let mut child_of = HashSet::new();
        for ix in 0..count {
            child_of.extend(self.subviews(ix));
        }
        let roots: Vec<usize> = (0..count)
            .filter(|ix| self.is_view(*ix) && !child_of.contains(ix))
            .collect();
        let mut seen = HashSet::new();
        let windows: Vec<Json> = (0..count)
            .filter(|ix| self.class(*ix).contains("WindowTemplate"))
            .map(|ix| self.resolve(ix, 2, &mut seen))
            .collect();
        let mut submenus = HashSet::new();
        for ix in 0..count {
            if let Some(ValueVariant::ObjectRef(r)) = self.field(ix, "NSSubmenu") {
                submenus.insert(*r as usize);
            }
        }
        let menus: Vec<Json> = (0..count)
            .filter(|ix| self.class(*ix) == "NSMenu" && !submenus.contains(ix))
            .map(|ix| self.menu_tree(ix))
            .collect();
        json!({
            "nib": name,
            "windows": windows,
            "menus": menus,
            "views": roots.into_iter().map(|ix| self.view_tree(ix, &mut seen)).collect::<Vec<_>>(),
        })
    }

    fn string_field(&self, ix: usize, key: &str) -> Option<String> {
        match self.field(ix, key)? {
            ValueVariant::ObjectRef(r) => self.as_string(*r as usize),
            _ => None,
        }
    }

    fn menu_tree(&self, ix: usize) -> Json {
        let items = match self.field(ix, "NSMenuItems") {
            Some(ValueVariant::ObjectRef(r)) => self.array_items(*r as usize),
            _ => Vec::new(),
        };
        let items: Vec<Json> = items
            .into_iter()
            .map(|item| {
                let mut node = Map::new();
                node.insert("title".into(), json!(self.string_field(item, "NSTitle")));
                if let Some(key) = self.string_field(item, "NSKeyEquiv").filter(|k| !k.is_empty()) {
                    node.insert("key".into(), json!(key));
                    if let Some(mods) = self.field(item, "NSKeyEquivModMask") {
                        node.insert("mods".into(), self.primitive(mods));
                    }
                }
                if matches!(self.field(item, "NSIsSeparator"), Some(ValueVariant::Bool(true))) {
                    node.insert("separator".into(), json!(true));
                }
                if let Some(ValueVariant::ObjectRef(sub)) = self.field(item, "NSSubmenu") {
                    node.insert("submenu".into(), self.menu_tree(*sub as usize));
                }
                Json::Object(node)
            })
            .collect();
        json!({ "title": self.string_field(ix, "NSTitle"), "items": items })
    }
}

fn help_index(app: &std::path::Path, out: Option<PathBuf>) -> Result<()> {
    let src = app.join("Contents/Resources/zh-Hans.lproj/Navicat Help");
    let out = out.unwrap_or_else(|| PathBuf::from("reference/help-index.json"));
    let mut pages: Vec<(String, String)> = Vec::new();
    for entry in fs::read_dir(&src).map_err(|e| format!("{}: {e}", src.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_none_or(|x| x != "html") {
            continue;
        }
        let html = fs::read_to_string(&path).unwrap_or_default();
        let title = extract_title(&html).unwrap_or_default();
        pages.push((path.file_name().unwrap().to_string_lossy().into(), title));
    }
    pages.sort();
    let list: Vec<Json> = pages
        .iter()
        .map(|(file, title)| json!({ "file": file, "title": title }))
        .collect();
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&out, serde_json::to_string_pretty(&list).unwrap()).map_err(|e| e.to_string())?;
    println!("整理 {} 个帮助页到 {}", list.len(), out.display());
    Ok(())
}

fn nib_tree(name: &str) -> Result<()> {
    let path = PathBuf::from(format!("reference/nib/{name}.json"));
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e}（先运行 cargo xtask nib-dump）", path.display()))?;
    let doc: Json = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    for w in doc["windows"].as_array().into_iter().flatten() {
        println!(
            "Window rect={} title={} toolbarStyle={}",
            w["NSWindowRect"], w["NSWindowTitle"], w["NSWindowToolbarStyle"]
        );
    }
    for m in doc["menus"].as_array().into_iter().flatten() {
        print_menu(m, 0);
    }
    for v in doc["views"].as_array().into_iter().flatten() {
        print_node(v, 0);
    }
    Ok(())
}

fn print_menu(menu: &Json, depth: usize) {
    println!("{}Menu {}", "  ".repeat(depth), menu["title"]);
    for item in menu["items"].as_array().into_iter().flatten() {
        if item["separator"] == json!(true) {
            println!("{}  ----", "  ".repeat(depth));
            continue;
        }
        let key = match item["key"].as_str() {
            Some(k) => format!("  ⌘{k} mods={}", item["mods"]),
            None => String::new(),
        };
        println!("{}  - {}{key}", "  ".repeat(depth), item["title"].as_str().unwrap_or(""));
        if item.get("submenu").is_some() {
            print_menu(&item["submenu"], depth + 2);
        }
    }
}

fn print_node(node: &Json, depth: usize) {
    let attrs = &node["attrs"];
    let frame = attrs
        .get("NSFrame")
        .or_else(|| attrs.get("NSFrameSize"))
        .and_then(Json::as_str)
        .unwrap_or("");
    let mut line = format!("{}{} {frame}", "  ".repeat(depth), node["class"].as_str().unwrap_or("?"));
    let cell = &attrs["NSCell"];
    if let Some(s) = cell["NSContents"].as_str().filter(|s| !s.is_empty()) {
        line.push_str(&format!(" \"{s}\""));
    }
    if let Some(font) = cell["NSSupport"].as_object() {
        line.push_str(&format!(" [{} {}]", font["NSName"], font["NSSize"]));
    }
    for key in ["NSTitle", "NSLabel", "NSIdentifier"] {
        if let Some(s) = attrs[key].as_str().filter(|s| !s.is_empty()) {
            line.push_str(&format!(" {key}={s}"));
        }
    }
    println!("{line}");
    for child in node["subviews"].as_array().into_iter().flatten() {
        print_node(child, depth + 1);
    }
}

fn extract_title(html: &str) -> Option<String> {
    let start = html.find("<title>")? + "<title>".len();
    let end = html[start..].find("</title>")? + start;
    Some(html[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::extract_title;

    #[test]
    fn extracts_title() {
        assert_eq!(
            extract_title("<html><title> 主窗口 </title></html>").as_deref(),
            Some("主窗口")
        );
        assert_eq!(extract_title("<html></html>"), None);
    }
}
