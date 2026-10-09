use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

// Lucide icons used by the toolbar, navigator and so on that are not among gpui-kit's default 101
icon_assets!(pub ExtraIcons, [
    EyeOff, Scan,
    Plug, SquareTerminal, Table, Eye, SquareFunction, User, Shapes, FileCode,
    DatabaseBackup, CalendarClock, Workflow, ChartPie, Database, Folder, FolderOpen,
    Server, Star, RefreshCw, Layers, Maximize2, Minimize2, CircleCheck, TriangleAlert,
    Plus, Minus, X, Search, Cable, Unplug, Palette, FolderInput, Copy, Trash, Pencil,
    Eraser, Scissors, List, TableProperties, Zap, KeyRound, ListOrdered, CircleX, Info, Code,
    Columns3, Check, RotateCcw, Play, Square, ChevronFirst, ChevronLast, ChevronLeft, ChevronRight,
    Funnel, ArrowUpDown, Clipboard, Undo2, Save, FileText, GitCommitHorizontal, CircleStop,
    Hash, Binary, Braces, Key,
]);

/// Our own color icons, drawn with `img()` (`svg()` can only tint in one color).
const OWN: &[(&str, &[u8])] = &[
    (
        "icons/db/mysql.svg",
        include_bytes!("../assets/icons/db/mysql.svg"),
    ),
    (
        "icons/db/mariadb.svg",
        include_bytes!("../assets/icons/db/mariadb.svg"),
    ),
    (
        "icons/db/postgresql.svg",
        include_bytes!("../assets/icons/db/postgresql.svg"),
    ),
    (
        "icons/db/sqlite.svg",
        include_bytes!("../assets/icons/db/sqlite.svg"),
    ),
];

/// App assets: our own icons, then the picked Lucide icons, then gpui-kit's defaults.
/// `Assets` errors on unknown paths, so it must come last.
#[derive(Clone, Copy, Default)]
pub struct NavidogAssets;

impl AssetSource for NavidogAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = OWN.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items: Vec<SharedString> = OWN
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect();
        items.extend(ExtraIcons.list(path)?);
        items.extend(Assets.list(path)?);
        items.sort();
        items.dedup();
        Ok(items)
    }
}

/// An icon by Lucide name (`IconName` only covers the default 101; the rest come from `ExtraIcons`).
pub fn lucide(name: &str) -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().path(format!("icons/{name}.svg"))
}

/// Icon path of a database type.
pub fn driver_icon(driver: &str) -> &'static str {
    match driver {
        "mariadb" => "icons/db/mariadb.svg",
        "postgres" => "icons/db/postgresql.svg",
        "sqlite" => "icons/db/sqlite.svg",
        _ => "icons/db/mysql.svg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_own_lucide_and_default_icons() {
        let a = NavidogAssets;
        assert!(a.load("icons/db/sqlite.svg").unwrap().is_some());
        assert!(a.load("icons/plug.svg").unwrap().is_some());
        assert!(a.load("icons/check.svg").unwrap().is_some());
        assert!(a.list("icons/db/").unwrap().len() == 4);
    }

    /// A missing icon only logs an error and draws nothing, so check every name the sources use.
    // ponytail: finds `lucide("x")` and toolbar `button(id, "x", …)` literals; names built at
    // runtime (kind_icon and the like) are not covered
    #[test]
    fn every_icon_used_in_the_sources_loads() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut names = Vec::new();
        let mut dirs = vec![crates];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() && !path.ends_with("target") {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let src = std::fs::read_to_string(&path).unwrap();
                    for part in src.split("lucide(\"").skip(1) {
                        names.push(part.split('"').next().unwrap().to_string());
                    }
                    for part in src.split("button(\"").skip(1) {
                        // The second string argument of button(id, icon, label)
                        let mut fields = part.split('"');
                        if let (Some(_), Some(sep), Some(icon)) =
                            (fields.next(), fields.next(), fields.next())
                            && sep.trim() == ","
                        {
                            names.push(icon.to_string());
                        }
                    }
                }
            }
        }
        names.sort();
        names.dedup();
        assert!(names.len() > 20, "the scan found too few icons: {names:?}");
        let missing: Vec<_> = names
            .iter()
            .filter(|n| {
                NavidogAssets
                    .load(&format!("icons/{n}.svg"))
                    .ok()
                    .flatten()
                    .is_none()
            })
            .collect();
        assert!(missing.is_empty(), "add these to ExtraIcons: {missing:?}");
    }
}
