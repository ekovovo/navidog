//! Identifiers page of the info pane (30-query-editor.md §6, F-INFO-006): tables and views of
//! the active query editor's database or schema, expanding to their columns. Double-click or
//! drag puts a name into the editor.

use std::collections::HashSet;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div,
};
use navidog_core::actions::InsertText;
use navidog_core::workspace::Workspace;
use navidog_db::dialect::Dialect;
use navidog_db::outline::{OutlineCache, OutlineState};
use navidog_db::sessions::Sessions;
use navidog_ui::assets::lucide;
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::info::InfoPanel;
use crate::snippets::DragLabel;

pub(crate) struct IdentifiersPage {
    pub search: Entity<InputState>,
    pub expanded: HashSet<String>,
}

impl IdentifiersPage {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            search: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("snippet.search").to_string())
            }),
            expanded: HashSet::new(),
        }
    }
}

enum Row {
    Table {
        name: String,
        view: bool,
        open: bool,
    },
    Column {
        name: String,
        ty: String,
    },
}

impl InfoPanel {
    /// Loads the completion info the page shows (once; the query editor refreshes it).
    pub(crate) fn ensure_identifiers(&self, cx: &mut App) {
        let Some(scope) = Workspace::global(cx)
            .read(cx)
            .query_context()
            .and_then(|c| c.scope())
        else {
            return;
        };
        if !Sessions::global(cx)
            .read(cx)
            .state(&scope.connection)
            .is_open()
        {
            return;
        }
        OutlineCache::global(cx).update(cx, |c, cx| c.ensure(&scope, false, cx));
    }

    pub(crate) fn render_identifiers(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let scope = Workspace::global(cx)
            .read(cx)
            .query_context()
            .and_then(|c| c.scope());
        let dialect = scope
            .as_ref()
            .and_then(|s| Sessions::global(cx).read(cx).dialect(&s.connection))
            .unwrap_or(Dialect::MySql);
        let needle = self.identifiers.search.read(cx).value().to_lowercase();
        let state = scope
            .as_ref()
            .and_then(|s| OutlineCache::global(cx).read(cx).get(s).cloned());
        let body: AnyElement = match state {
            Some(OutlineState::Ready(outline)) => {
                let mut rows = Vec::new();
                for t in &outline.tables {
                    let cols: Vec<_> = t
                        .columns
                        .iter()
                        .filter(|(c, _)| needle.is_empty() || c.to_lowercase().contains(&needle))
                        .collect();
                    let hit = needle.is_empty() || t.name.to_lowercase().contains(&needle);
                    if !hit && cols.is_empty() {
                        continue;
                    }
                    let open = self.identifiers.expanded.contains(&t.name)
                        || (!needle.is_empty() && !cols.is_empty());
                    rows.push(Row::Table {
                        name: t.name.clone(),
                        view: t.view,
                        open,
                    });
                    if open {
                        let shown: Vec<_> = if hit {
                            t.columns.iter().collect()
                        } else {
                            cols
                        };
                        rows.extend(shown.into_iter().map(|(n, ty)| Row::Column {
                            name: n.clone(),
                            ty: ty.clone(),
                        }));
                    }
                }
                v_flex()
                    .id("identifier-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows.into_iter().enumerate().map(|(ix, row)| {
                        let (name, indent, icon, detail, table) = match &row {
                            Row::Table { name, view, open } => (
                                name.clone(),
                                false,
                                if *view { "eye" } else { "table" },
                                None,
                                Some((name.clone(), *open)),
                            ),
                            Row::Column { name, ty } => {
                                (name.clone(), true, "columns-3", Some(ty.clone()), None)
                            }
                        };
                        let typed: SharedString = dialect.typed_name(&name).into();
                        let insert = InsertText {
                            text: typed.clone(),
                            snippet: false,
                        };
                        let label: SharedString = name.clone().into();
                        h_flex()
                            .id(("identifier", ix))
                            .h(tokens::navigator::ROW_HEIGHT.rems())
                            .px_2()
                            .gap_1()
                            .min_w_0()
                            .when(indent, |d| d.pl(tokens::navigator::ROW_INDENT.rems() * 2.))
                            .children(table.as_ref().map(|(_, open)| {
                                lucide(if *open {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                })
                                .size(tokens::navigator::ROW_ICON.rems())
                            }))
                            .child(lucide(icon).size(tokens::navigator::ROW_ICON.rems()))
                            .child(truncated(("identifier-name", ix), label.clone()).flex_1())
                            .children(detail.map(|d| {
                                truncated(("identifier-type", ix), d)
                                    .text_size(tokens::font::SIZE_SMALL.rems())
                                    .text_color(p.label_secondary)
                            }))
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                                if e.click_count() == 2 {
                                    window.dispatch_action(Box::new(insert.clone()), cx);
                                } else if let Some((t, _)) = &table {
                                    if !this.identifiers.expanded.remove(t) {
                                        this.identifiers.expanded.insert(t.clone());
                                    }
                                    cx.notify();
                                }
                            }))
                            .on_drag(
                                InsertText {
                                    text: typed,
                                    snippet: false,
                                },
                                move |_, _, _, cx| cx.new(|_| DragLabel(label.clone())),
                            )
                    }))
                    .into_any_element()
            }
            Some(OutlineState::Failed(e)) => self.message(e, cx),
            _ if scope.is_none() => self.message(t!("snippet.no_database").to_string(), cx),
            _ => self.message(t!("nav.loading").to_string(), cx),
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(tokens::info::SEARCH_BAR_HEIGHT.rems())
                    .px_2()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        div()
                            .flex_1()
                            .h(tokens::info::SEARCH_FIELD_HEIGHT.rems())
                            .child(Input::new(&self.identifiers.search).small().cleanable(true)),
                    ),
            )
            .child(body)
            .into_any_element()
    }
}
