//! Code completion (30-query-editor.md §4): keywords and functions of the dialect, the database's
//! tables, views, functions and columns from the code completion info, and snippets by name.
//! After `table.` or `alias.` only that table's columns are offered; after `schema.` its tables.
//! ponytail: aliases come from `FROM`/`JOIN` in the current statement only, without sub-query
//! scopes; a syntax tree would handle those.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use gpui_kit::component::input::{
    CompletionProvider, Rope, RopeExt as _, highlight_characters, highlight_contains,
};
use gpui_kit::{App, Context, Task, WeakEntity, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionResponse, CompletionTextEdit, TextEdit,
};
use navidog_core::settings::Settings;
use navidog_core::snippets::Snippets;
use navidog_core::store::ConnectionStore;
use navidog_db::dialect::Dialect;
use navidog_db::outline::{Outline, OutlineCache};
use regex::Regex;
use rust_i18n::t;

use crate::editor::QueryEditor;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Candidate {
    pub label: String,
    pub detail: String,
    pub insert: String,
}

pub(crate) struct Sources<'a> {
    pub dialect: Dialect,
    /// Database (PostgreSQL: schema) the outline belongs to
    pub current: Option<&'a str>,
    pub outline: Option<&'a Outline>,
    /// Outlines of other databases or schemas that are already loaded
    pub others: &'a dyn Fn(&str) -> Option<Arc<Outline>>,
    pub snippets: &'a [(String, String)],
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    for q in ['`', '"'] {
        if let Some(inner) = t.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner.to_string();
        }
    }
    t.to_string()
}

pub(crate) fn typed_name(dialect: Dialect, name: &str) -> String {
    dialect.typed_name(name)
}

static ALIAS: LazyLock<Regex> = LazyLock::new(|| {
    let ident = r#"(?:[\w$]+|"[^"]+"|`[^`]+`)"#;
    Regex::new(&format!(
        r"(?i)\b(?:from|join|update|into)\s+({ident}(?:\s*\.\s*{ident})?)(?:\s+(?:as\s+)?({ident}))?"
    ))
    .expect("alias pattern")
});

/// Table names and aliases in the statement around `offset`, alias (or table) → table.
fn aliases(statement: &str, dialect: Dialect) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for caps in ALIAS.captures_iter(statement) {
        let full = caps[1].to_string();
        let table = unquote(full.rsplit('.').next().unwrap_or(&full));
        out.insert(table.to_lowercase(), table.clone());
        if let Some(a) = caps.get(2) {
            let alias = unquote(a.as_str());
            if !dialect.is_keyword(&alias) {
                out.insert(alias.to_lowercase(), table);
            }
        }
    }
    out
}

/// The word being typed and what to offer for it.
pub(crate) fn complete(text: &str, offset: usize, src: &Sources) -> (Range<usize>, Vec<Candidate>) {
    let before = &text[..offset];
    let word_len: usize = before
        .chars()
        .rev()
        .take_while(|c| is_ident(*c))
        .map(char::len_utf8)
        .sum();
    let start = offset - word_len;
    let word = &text[start..offset];
    let stmt_start = text[..start].rfind(';').map_or(0, |i| i + 1);
    let stmt_end = text[offset..].find(';').map_or(text.len(), |i| offset + i);
    let statement = &text[stmt_start..stmt_end];
    let known = aliases(statement, src.dialect);
    let column = |name: &str, ty: &str| Candidate {
        label: name.to_string(),
        detail: format!("{} · {ty}", t!("query.kind_column")),
        insert: typed_name(src.dialect, name),
    };
    let tables_of = |outline: &Outline| -> Vec<Candidate> {
        outline
            .tables
            .iter()
            .map(|t| Candidate {
                label: t.name.clone(),
                detail: if t.view {
                    t!("query.kind_view").to_string()
                } else {
                    t!("query.kind_table").to_string()
                },
                insert: typed_name(src.dialect, &t.name),
            })
            .collect()
    };
    let mut out: Vec<Candidate> = Vec::new();
    let qualifier = before[..start].strip_suffix('.').map(|q| {
        let len: usize = q
            .chars()
            .rev()
            .take_while(|c| is_ident(*c) || *c == '`' || *c == '"')
            .map(char::len_utf8)
            .sum();
        unquote(&q[q.len() - len..])
    });
    if let Some(q) = qualifier {
        let table = known.get(&q.to_lowercase()).cloned().unwrap_or(q.clone());
        if let Some(t) = src.outline.and_then(|o| o.table(&table)) {
            out.extend(t.columns.iter().map(|(n, ty)| column(n, ty)));
        } else if src.current.is_some_and(|c| c.eq_ignore_ascii_case(&q)) {
            out.extend(src.outline.map(tables_of).unwrap_or_default());
        } else if let Some(o) = (src.others)(&q) {
            out.extend(tables_of(&o));
        }
    } else {
        if let Some(o) = src.outline {
            for table in known.values() {
                if let Some(t) = o.table(table) {
                    out.extend(t.columns.iter().map(|(n, ty)| column(n, ty)));
                }
            }
            out.extend(tables_of(o));
            out.extend(o.schemas.iter().map(|s| Candidate {
                label: s.clone(),
                detail: if src.dialect.has_schemas() {
                    t!("query.schema").to_string()
                } else {
                    t!("query.database").to_string()
                },
                insert: typed_name(src.dialect, s),
            }));
            out.extend(o.functions.iter().map(|f| Candidate {
                label: f.clone(),
                detail: t!("query.kind_function").to_string(),
                insert: typed_name(src.dialect, f),
            }));
        }
        out.extend(src.snippets.iter().map(|(name, code)| Candidate {
            label: name.clone(),
            detail: t!("query.snippets").to_string(),
            insert: code.clone(),
        }));
        out.extend(src.dialect.functions().map(|f| Candidate {
            label: f.to_string(),
            detail: t!("query.kind_builtin").to_string(),
            insert: f.to_string(),
        }));
        out.extend(src.dialect.keywords().map(|k| Candidate {
            label: k.to_string(),
            detail: t!("query.kind_keyword").to_string(),
            insert: k.to_string(),
        }));
    }
    let needle = word.to_lowercase();
    let mut seen = std::collections::HashSet::new();
    let mut ranked: Vec<(u8, u32, Candidate)> = out
        .into_iter()
        .filter(|c| seen.insert((c.label.clone(), c.detail.clone())))
        .filter_map(|c| {
            let label = c.label.to_lowercase();
            match_key(&label, &needle).map(|(tier, gaps)| (tier, gaps, c))
        })
        .collect();
    ranked.sort_by(|(ta, ga, a), (tb, gb, b)| {
        ta.cmp(tb)
            .then(ga.cmp(gb))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    (
        start..offset,
        ranked.into_iter().map(|(_, _, c)| c).collect(),
    )
}

/// Prefix is 0, a consecutive substring is 1, a subsequence is 2.
/// `gaps` is characters skipped between the first and last hit; only
/// subsequences use it, so a tighter match sorts first.
fn match_key(label: &str, needle: &str) -> Option<(u8, u32)> {
    if needle.is_empty() || label.starts_with(needle) {
        return Some((0, 0));
    }
    if label.contains(needle) {
        return Some((1, 0));
    }
    subsequence_gaps(label, needle).map(|gaps| (2, gaps))
}

/// Characters between the first and last matched character, minus the needle.
/// `None` when `needle` is not a subsequence of `label`.
fn subsequence_gaps(label: &str, needle: &str) -> Option<u32> {
    let mut rest = label.chars().enumerate();
    let mut first = None;
    let mut last = 0;
    for n in needle.chars() {
        let (i, _) = rest.find(|(_, c)| *c == n)?;
        first.get_or_insert(i);
        last = i;
    }
    let span = last - first? + 1;
    Some((span - needle.chars().count()) as u32)
}

/// Byte ranges the completion menu paints. A contiguous hit (prefix or
/// contains) wins, so `info` in `in_info` paints that span rather than the
/// looser characters around it. Otherwise each matched character is painted,
/// and adjacent hits merge into one range.
pub(crate) fn highlight_label(query: &str, item: &CompletionItem) -> Vec<Range<usize>> {
    let contiguous = highlight_contains(query, item);
    if contiguous.is_empty() {
        highlight_characters(query, item)
    } else {
        contiguous
    }
}

/// Feeds the editor's completion menu.
pub(crate) struct Provider {
    pub editor: WeakEntity<QueryEditor>,
    /// Snippet codes offered last, to spot one being inserted and start its tab stops
    pub offered: Rc<RefCell<Vec<String>>>,
}

/// Completion reads at most this much text before the caret.
const WINDOW: usize = 64 * 1024;

impl CompletionProvider for Provider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let Some(editor) = self.editor.upgrade() else {
            return Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        };
        let e = editor.read(cx);
        let dialect = e.dialect(cx).unwrap_or(Dialect::MySql);
        let scope = e.target.scope();
        let driver = e
            .target
            .connection
            .as_ref()
            .and_then(|id| ConnectionStore::global(cx).read(cx).get(id))
            .map(|c| c.driver.clone())
            .unwrap_or_default();
        let cache = OutlineCache::global(cx);
        let cache = cache.read(cx);
        let outline = scope.as_ref().and_then(|s| cache.ready(s));
        let others = |name: &str| {
            let s = scope.as_ref()?;
            let other = if dialect.has_schemas() {
                navidog_core::object::Scope::new(
                    s.connection.clone(),
                    s.database.clone(),
                    Some(name.into()),
                )
            } else {
                navidog_core::object::Scope::new(s.connection.clone(), name, None)
            };
            cache.ready(&other)
        };
        let snippets: Vec<(String, String)> = Snippets::global(cx)
            .read(cx)
            .all()
            .filter(|s| s.applies_to(&driver))
            .map(|s| (s.name.clone(), s.code.clone()))
            .collect();
        let current = if dialect.has_schemas() {
            e.target.schema.clone()
        } else {
            e.target.database.clone()
        };
        let from = offset.saturating_sub(WINDOW);
        let from = (from..=offset)
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(offset);
        let to = (offset + 4096).min(text.len());
        let to = (to..=text.len())
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(text.len());
        let window_text = text.slice(from..to).to_string();
        let (range, list) = complete(
            &window_text,
            offset - from,
            &Sources {
                dialect,
                current: current.as_deref(),
                outline: outline.as_deref(),
                others: &others,
                snippets: &snippets,
            },
        );
        let snippet_codes: Vec<String> = snippets.into_iter().map(|(_, code)| code).collect();
        *self.offered.borrow_mut() = snippet_codes;
        let lsp_range = lsp_types::Range {
            start: text.offset_to_position(range.start + from),
            end: text.offset_to_position(range.end + from),
        };
        let items = list
            .into_iter()
            .take(200)
            .map(|c| CompletionItem {
                label: c.label,
                detail: Some(c.detail),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: lsp_range,
                    new_text: c.insert,
                })),
                ..CompletionItem::default()
            })
            .collect();
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, cx: &mut App) -> bool {
        let Some(editor) = self.editor.upgrade() else {
            return false;
        };
        // Runs inside the editor update (each keystroke). `completion_on` must
        // not read that editor, or the app aborts.
        completion_on(editor.read(cx), cx)
            && !new_text.is_empty()
            && new_text.chars().all(|c| is_ident(c) || c == '.')
    }
}

/// Off in the preferences, or above the size where highlighting and completion stop (F-PREF-004).
pub(crate) fn completion_on(e: &QueryEditor, cx: &App) -> bool {
    Settings::global(cx).code_completion && e.sql_highlight
}

impl QueryEditor {
    /// Loads the completion info for the picked database or schema; with "auto update" it is
    /// fetched again each time one is picked.
    pub(crate) fn ensure_outline(&mut self, refresh: bool, cx: &mut Context<Self>) {
        let Some(scope) = self.target.scope() else {
            return;
        };
        if !navidog_db::sessions::Sessions::global(cx)
            .read(cx)
            .state(&scope.connection)
            .is_open()
        {
            return;
        }
        let auto = Settings::global(cx).auto_update_outline;
        OutlineCache::global(cx).update(cx, |c, cx| c.ensure(&scope, refresh || auto, cx));
    }

    /// A snippet chosen from the completion menu arrives as its raw code; turn its `${…}` into
    /// tab stops.
    pub(crate) fn expand_inserted_snippet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let codes = self.offered_snippets.borrow().clone();
        if codes.is_empty() {
            return;
        }
        let (caret, text) = {
            let e = self.editor.read(cx);
            (e.cursor(), e.value().to_string())
        };
        let hit = codes.into_iter().find(|code| {
            code.contains("${")
                && caret >= code.len()
                && text.get(caret - code.len()..caret) == Some(code.as_str())
        });
        let Some(code) = hit else { return };
        self.offered_snippets.borrow_mut().clear();
        let start = caret - code.len();
        self.editor
            .update(cx, |e, cx| e.set_selected_range(start..caret, cx));
        self.insert_snippet(&code, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use navidog_db::outline::OutlineTable;

    fn outline() -> Outline {
        Outline {
            schemas: vec!["sakila".into(), "world".into()],
            tables: vec![
                OutlineTable {
                    name: "actor".into(),
                    view: false,
                    columns: vec![
                        ("actor_id".into(), "smallint".into()),
                        ("first_name".into(), "varchar(45)".into()),
                    ],
                },
                OutlineTable {
                    name: "actor_info".into(),
                    view: true,
                    columns: vec![("actor_id".into(), "smallint".into())],
                },
            ],
            functions: vec!["get_customer_balance".into()],
        }
    }

    fn run(
        text: &str,
        dialect: Dialect,
        others: &dyn Fn(&str) -> Option<Arc<Outline>>,
    ) -> Vec<String> {
        let o = outline();
        let snippets = vec![("my snippet".to_string(), "SELECT ${x}".to_string())];
        let offset = text.find('|').unwrap();
        let text = text.replace('|', "");
        complete(
            &text,
            offset,
            &Sources {
                dialect,
                current: Some("sakila"),
                outline: Some(&o),
                others,
                snippets: &snippets,
            },
        )
        .1
        .into_iter()
        .map(|c| c.label)
        .collect()
    }

    #[test]
    fn dot_after_database_lists_tables_and_after_alias_lists_columns() {
        let none = |_: &str| None;
        assert_eq!(
            run("SELECT * FROM sakila.|", Dialect::MySql, &none),
            ["actor", "actor_info"]
        );
        assert_eq!(
            run("SELECT a.| FROM actor a WHERE 1", Dialect::MySql, &none),
            ["actor_id", "first_name"]
        );
        assert_eq!(
            run("SELECT x.f| FROM actor AS x", Dialect::MySql, &none),
            ["first_name"]
        );
        assert_eq!(
            run("SELECT actor.| FROM actor", Dialect::MySql, &none),
            ["actor_id", "first_name"]
        );
        let world = |name: &str| {
            (name == "world").then(|| {
                Arc::new(Outline {
                    tables: vec![OutlineTable {
                        name: "city".into(),
                        ..OutlineTable::default()
                    }],
                    ..Outline::default()
                })
            })
        };
        assert_eq!(
            run("SELECT * FROM world.|", Dialect::MySql, &world),
            ["city"]
        );
    }

    #[test]
    fn prefix_matches_come_before_contains_and_snippets_show_up() {
        let none = |_: &str| None;
        let got = run("SELECT * FROM act|", Dialect::MySql, &none);
        assert_eq!(&got[..2], ["actor", "actor_info"]);
        assert!(run("SELECT * FROM t WHERE first|", Dialect::MySql, &none).is_empty());
        assert!(
            run("SELECT first| FROM actor", Dialect::MySql, &none)
                .contains(&"first_name".to_string())
        );
        assert!(run("my|", Dialect::Sqlite, &none).contains(&"my snippet".to_string()));
        assert!(run("sel|", Dialect::Postgres, &none).contains(&"SELECT".to_string()));
        // `info` is inside `actor_info`, after the prefix matches
        let got = run("info|", Dialect::MySql, &none);
        assert!(got.iter().position(|l| l == "actor_info").is_some());
    }

    #[test]
    fn subsequence_matches_rank_after_tighter_hits() {
        let outline = Outline {
            tables: vec![
                OutlineTable {
                    name: "mocfg".into(),
                    ..OutlineTable::default()
                },
                OutlineTable {
                    name: "x_mocfg_y".into(),
                    ..OutlineTable::default()
                },
                OutlineTable {
                    name: "mount_cfgs".into(),
                    ..OutlineTable::default()
                },
                OutlineTable {
                    name: "m_x_o_x_c_f_g".into(),
                    ..OutlineTable::default()
                },
            ],
            ..Outline::default()
        };
        let labels = |text: &str| {
            let offset = text.find('|').unwrap();
            let text = text.replace('|', "");
            let none = |_: &str| None;
            complete(
                &text,
                offset,
                &Sources {
                    dialect: Dialect::Sqlite,
                    current: None,
                    outline: Some(&outline),
                    others: &none,
                    snippets: &[],
                },
            )
            .1
            .into_iter()
            .map(|c| c.label)
            .collect::<Vec<_>>()
        };
        let got = labels("mocfg|");
        let pos = |name: &str| got.iter().position(|l| l == name).unwrap();
        assert!(pos("mocfg") < pos("x_mocfg_y"));
        assert!(pos("x_mocfg_y") < pos("mount_cfgs"));
        assert!(pos("mount_cfgs") < pos("m_x_o_x_c_f_g"));
        assert!(labels("MOCFG|").contains(&"mount_cfgs".to_string()));
        assert!(labels("munt|").contains(&"mount_cfgs".to_string()));
        assert!(!labels("moxnt|").iter().any(|l| l == "mount_cfgs"));
    }

    fn label(text: &str) -> CompletionItem {
        CompletionItem {
            label: text.into(),
            ..CompletionItem::default()
        }
    }

    #[test]
    fn highlight_paints_the_matched_characters() {
        assert_eq!(highlight_label("act", &label("actor")), vec![0..3]);
        assert_eq!(highlight_label("info", &label("actor_info")), vec![6..10]);
        assert_eq!(highlight_label("info", &label("in_info")), vec![3..7]);
        assert_eq!(
            highlight_label("mocfg", &label("mount_cfgs")),
            vec![0..2, 6..9]
        );
        assert_eq!(
            highlight_label("MOCFG", &label("mount_cfgs")),
            vec![0..2, 6..9]
        );
        assert!(highlight_label("moxnt", &label("mount_cfgs")).is_empty());
    }
}
