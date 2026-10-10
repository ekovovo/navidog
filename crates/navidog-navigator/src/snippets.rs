//! Code snippets page of the info pane and the snippet dialogs (30-query-editor.md §5, F-INFO-005).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, Input, InputState};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use navidog_core::actions::InsertText;
use navidog_core::snippets::{Snippet, Snippets};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::Workspace;
use navidog_ui::assets::lucide;
use navidog_ui::skin::{FormRow, heading, truncated};
use navidog_ui::{palette, tokens};
use navidog_util::sql_text::{add_placeholder, remove_placeholders};
use rust_i18n::t;

use crate::info::InfoPanel;

/// Snippet commands from the page's menus; they act on the snippet with this id.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = snippets, no_json)]
pub(crate) struct PreviewSnippet {
    pub id: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = snippets, no_json)]
pub(crate) struct EditSnippet {
    pub id: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = snippets, no_json)]
pub(crate) struct DuplicateSnippet {
    pub id: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = snippets, no_json)]
pub(crate) struct DeleteSnippet {
    pub id: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = snippets, no_json)]
pub(crate) struct FilterTag {
    /// Empty is "All Tags"
    pub tag: SharedString,
}

gpui_kit::actions!(
    snippets,
    [NewSnippet, ToggleOtherTypes, TogglePresets, RefreshSnippets]
);

/// Database types a snippet can be for: driver id, label.
const TYPES: &[(&str, &str)] = &[
    ("", "snippet.all_types"),
    ("mysql", "MySQL"),
    ("mariadb", "MariaDB"),
    ("postgres", "PostgreSQL"),
    ("sqlite", "SQLite"),
];

fn type_label(driver: &str) -> String {
    TYPES
        .iter()
        .find(|(d, _)| *d == driver)
        .map(|(_, l)| {
            if l.starts_with("snippet.") {
                t!(*l).to_string()
            } else {
                l.to_string()
            }
        })
        .unwrap_or_else(|| driver.to_string())
}

pub(crate) struct SnippetsPage {
    pub search: Entity<InputState>,
    pub tag: Option<String>,
    pub show_other_types: bool,
    pub show_presets: bool,
    pub selected: Option<String>,
}

impl SnippetsPage {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            search: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("snippet.search").to_string())
            }),
            tag: None,
            show_other_types: true,
            show_presets: true,
            selected: None,
        }
    }
}

/// The driver of the active query editor's connection.
fn context_driver(cx: &App) -> Option<String> {
    let ctx = Workspace::global(cx).read(cx).query_context()?.clone();
    let id = ctx.connection?;
    Some(
        ConnectionStore::global(cx)
            .read(cx)
            .get(&id)?
            .driver
            .clone(),
    )
}

/// What a dragged snippet or identifier looks like under the pointer.
pub(crate) struct DragLabel(pub SharedString);

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(p.window_bg)
            .border_1()
            .border_color(p.separator)
            .text_color(p.label)
            .child(self.0.clone())
    }
}

impl InfoPanel {
    fn visible_snippets(&self, cx: &App) -> Vec<Snippet> {
        let page = &self.snippets;
        let driver = context_driver(cx).unwrap_or_default();
        let needle = page.search.read(cx).value().to_lowercase();
        Snippets::global(cx)
            .read(cx)
            .all()
            .filter(|s| page.show_other_types || s.applies_to(&driver))
            .filter(|s| page.show_presets || !s.preset)
            .filter(|s| page.tag.as_ref().is_none_or(|t| s.tags.contains(t)))
            .filter(|s| {
                needle.is_empty()
                    || s.name.to_lowercase().contains(&needle)
                    || s.code.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect()
    }

    fn snippet_menu(&self, id: Option<&str>) -> NativeMenu {
        let page = &self.snippets;
        let mut menu = NativeMenu::new();
        if let Some(id) = id {
            let id: SharedString = id.to_string().into();
            menu = menu
                .menu(
                    t!("snippet.preview").to_string(),
                    Box::new(PreviewSnippet { id: id.clone() }),
                )
                .separator()
                .menu(t!("snippet.new").to_string(), Box::new(NewSnippet))
                .menu(
                    t!("snippet.edit").to_string(),
                    Box::new(EditSnippet { id: id.clone() }),
                )
                .menu(
                    t!("snippet.duplicate").to_string(),
                    Box::new(DuplicateSnippet { id: id.clone() }),
                )
                .menu(
                    t!("snippet.delete").to_string(),
                    Box::new(DeleteSnippet { id }),
                )
                .separator();
        } else {
            menu = menu
                .menu(t!("snippet.new").to_string(), Box::new(NewSnippet))
                .separator();
        }
        menu.menu_with_check(
            t!("snippet.show_other_types").to_string(),
            page.show_other_types,
            Box::new(ToggleOtherTypes),
        )
        .menu_with_check(
            t!("snippet.show_presets").to_string(),
            page.show_presets,
            Box::new(TogglePresets),
        )
        .separator()
        .menu(t!("snippet.refresh").to_string(), Box::new(RefreshSnippets))
    }

    pub(crate) fn render_snippets(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let tags = Snippets::global(cx).read(cx).tags();
        let list = self.visible_snippets(cx);
        let selected = self.snippets.selected.clone();
        let tag_label = self
            .snippets
            .tag
            .clone()
            .unwrap_or_else(|| t!("snippet.all_tags").to_string());
        v_flex()
            .size_full()
            .on_action(cx.listener(|this, a: &FilterTag, _, cx| {
                this.snippets.tag = (!a.tag.is_empty()).then(|| a.tag.to_string());
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleOtherTypes, _, cx| {
                this.snippets.show_other_types = !this.snippets.show_other_types;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TogglePresets, _, cx| {
                this.snippets.show_presets = !this.snippets.show_presets;
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &RefreshSnippets, _, cx| cx.notify()))
            .on_action(|_: &NewSnippet, window, cx| open_editor(None, String::new(), window, cx))
            .on_action(|a: &PreviewSnippet, window, cx| open_viewer(&a.id, window, cx))
            .on_action(|a: &EditSnippet, window, cx| {
                let found = Snippets::global(cx).read(cx).get(&a.id).cloned();
                if let Some(s) = found.filter(|s| !s.preset) {
                    let code = s.code.clone();
                    open_editor(Some(s), code, window, cx);
                }
            })
            .on_action(cx.listener(|this, a: &DuplicateSnippet, _, cx| {
                let copy = Snippets::global(cx).update(cx, |s, cx| s.duplicate(&a.id, cx));
                this.snippets.selected = copy;
                cx.notify();
            }))
            .on_action(|a: &DeleteSnippet, window, cx| {
                let found = Snippets::global(cx).read(cx).get(&a.id).cloned();
                let Some(s) = found.filter(|s| !s.preset) else {
                    return;
                };
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let id = s.id.clone();
                    alert
                        .icon(lucide("triangle-alert"))
                        .title(t!("snippet.confirm_delete", name = s.name).to_string())
                        .show_cancel(true)
                        .on_ok(move |_, _, cx| {
                            Snippets::global(cx).update(cx, |s, cx| s.delete(&id, cx));
                            true
                        })
                });
            })
            .child(
                h_flex()
                    .h(tokens::snippet::FILTER_HEIGHT.rems())
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        Button::new("snippet-tags")
                            .ghost()
                            .xsmall()
                            .label(tag_label)
                            .icon(lucide("chevron-down"))
                            .on_click(move |e: &ClickEvent, window, cx| {
                                let menu = tags.iter().fold(
                                    NativeMenu::new()
                                        .menu(
                                            t!("snippet.all_tags").to_string(),
                                            Box::new(FilterTag {
                                                tag: SharedString::default(),
                                            }),
                                        )
                                        .separator(),
                                    |m, tag| {
                                        m.menu(
                                            tag.clone(),
                                            Box::new(FilterTag {
                                                tag: tag.clone().into(),
                                            }),
                                        )
                                    },
                                );
                                menu.show(e.position(), window, cx);
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("snippet-new")
                            .ghost()
                            .xsmall()
                            .icon(lucide("plus"))
                            .tooltip(t!("snippet.new").to_string())
                            .on_click(|_, window, cx| open_editor(None, String::new(), window, cx)),
                    ),
            )
            .child(
                v_flex()
                    .id("snippet-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, e: &gpui_kit::MouseDownEvent, window, cx| {
                            this.focus_for_menu(window, cx);
                            this.snippet_menu(None).show(e.position, window, cx);
                        }),
                    )
                    .children(list.into_iter().map(|s| {
                        let id = s.id.clone();
                        let (click_id, menu_id) = (id.clone(), id.clone());
                        let insert = InsertText {
                            text: s.code.clone().into(),
                            snippet: true,
                        };
                        let name: SharedString = s.name.clone().into();
                        let detail = match (s.preset, s.driver.is_empty()) {
                            (true, _) => {
                                format!("{} · {}", t!("snippet.preset"), type_label(&s.driver))
                            }
                            (false, _) => type_label(&s.driver),
                        };
                        h_flex()
                            .id(SharedString::from(format!("snippet-{id}")))
                            .h(tokens::snippet::ROW_HEIGHT.rems())
                            .px_2()
                            .gap_2()
                            .min_w_0()
                            .when(selected.as_deref() == Some(id.as_str()), |d| {
                                d.bg(p.row_selected)
                            })
                            .child(
                                lucide("scroll-text")
                                    .size(tokens::snippet::ROW_ICON.rems())
                                    .text_color(p.icon_teal),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(truncated(
                                        SharedString::from(format!("snippet-name-{id}")),
                                        name.clone(),
                                    ))
                                    .child(
                                        truncated(
                                            SharedString::from(format!("snippet-detail-{id}")),
                                            detail,
                                        )
                                        .text_size(tokens::font::SIZE_SMALL.rems())
                                        .text_color(p.label_secondary),
                                    ),
                            )
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                                this.snippets.selected = Some(click_id.clone());
                                cx.notify();
                                if e.click_count() == 2 {
                                    open_viewer(&click_id, window, cx);
                                }
                            }))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(
                                    move |this, e: &gpui_kit::MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.snippets.selected = Some(menu_id.clone());
                                        this.focus_for_menu(window, cx);
                                        this.snippet_menu(Some(&menu_id))
                                            .show(e.position, window, cx);
                                    },
                                ),
                            )
                            .on_drag(insert, move |_, _, _, cx| {
                                cx.new(|_| DragLabel(name.clone()))
                            })
                    })),
            )
            .child(
                div()
                    .p_1()
                    .border_t_1()
                    .border_color(p.separator)
                    .child(Input::new(&self.snippets.search).small().cleanable(true)),
            )
            .into_any_element()
    }
}

/// Double-click: the snippet's name and code, with "Edit" for custom ones (360 × 341).
pub(crate) fn open_viewer(id: &str, window: &mut Window, cx: &mut App) {
    let Some(s) = Snippets::global(cx).read(cx).get(id).cloned() else {
        return;
    };
    let code = cx.new(|cx| {
        EditorState::new(window, cx)
            .language("sql")
            .line_number(false)
    });
    code.update(cx, |e, cx| {
        e.set_value(s.code.clone(), window, cx);
        e.set_readonly(true, cx);
    });
    window.open_dialog(cx, move |dialog, window, _| {
        let s = s.clone();
        let editable = !s.preset;
        let edit = s.clone();
        dialog
            .w(tokens::snippet::POPOVER_WIDTH.to_pixels(window.rem_size()))
            .title(s.name.clone())
            .child(
                div()
                    .h(tokens::snippet::CODE_HEIGHT.rems())
                    .child(Editor::new(&code).readonly(true).size_full()),
            )
            .footer(h_flex().w_full().justify_end().gap_2().when(editable, |f| {
                f.child(
                    Button::new("snippet-view-edit")
                        .label(t!("snippet.edit").to_string())
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let code = edit.code.clone();
                            open_editor(Some(edit.clone()), code, window, cx);
                        }),
                )
            }))
    });
}

/// New or edit (360 × 507): name, database type, tags, note and code with placeholder buttons.
pub(crate) fn open_editor(
    existing: Option<Snippet>,
    code: String,
    window: &mut Window,
    cx: &mut App,
) {
    let s = existing.unwrap_or_else(|| Snippet {
        id: String::new(),
        name: String::new(),
        driver: context_driver(cx).unwrap_or_default(),
        tags: Vec::new(),
        note: String::new(),
        code: String::new(),
        preset: false,
    });
    let view = cx.new(|cx| {
        let name = cx.new(|cx| InputState::new(window, cx).default_value(s.name.clone()));
        // Save follows the name's validity as it is typed
        cx.observe(&name, |_, _, cx| cx.notify()).detach();
        SnippetEditor {
            name,
            tags: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(s.tags.join(", "))
                    .placeholder(t!("snippet.tags_hint").to_string())
            }),
            note: cx.new(|cx| InputState::new(window, cx).default_value(s.note.clone())),
            code: {
                let e = cx.new(|cx| {
                    EditorState::new(window, cx)
                        .language("sql")
                        .line_number(false)
                });
                e.update(cx, |e, cx| e.set_value(code, window, cx));
                e
            },
            driver: s.driver.clone(),
            id: s.id.clone(),
        }
    });
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(tokens::snippet::POPOVER_WIDTH.to_pixels(window.rem_size()))
            .close_button(false)
            .child(view.clone())
    });
}

struct SnippetEditor {
    id: String,
    name: Entity<InputState>,
    driver: String,
    tags: Entity<InputState>,
    note: Entity<InputState>,
    code: Entity<EditorState>,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = snippets, no_json)]
struct PickType {
    driver: SharedString,
}

impl SnippetEditor {
    fn name_ok(&self, cx: &App) -> bool {
        let name = self.name.read(cx).value().to_string();
        let except = (!self.id.is_empty()).then_some(self.id.as_str());
        Snippets::global(cx)
            .read(cx)
            .name_ok(&name, &self.driver, except)
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.name_ok(cx) {
            return;
        }
        let tags = self
            .tags
            .read(cx)
            .value()
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        let snippet = Snippet {
            id: self.id.clone(),
            name: self.name.read(cx).value().trim().to_string(),
            driver: self.driver.clone(),
            tags,
            note: self.note.read(cx).value().to_string(),
            code: self.code.read(cx).value().to_string(),
            preset: false,
        };
        Snippets::global(cx).update(cx, |s, cx| s.upsert(snippet, cx));
        window.close_dialog(cx);
    }

    fn rewrite_code(
        &mut self,
        f: impl FnOnce(&str, std::ops::Range<usize>) -> String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text, sel) = {
            let e = self.code.read(cx);
            (e.value().to_string(), e.selected_range())
        };
        let new = f(&text, sel);
        if new != text {
            let len = text.len();
            self.code.update(cx, |e, cx| {
                e.set_selected_range(0..len, cx);
                e.replace(new, window, cx);
            });
        }
    }
}

impl Render for SnippetEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ok = self.name_ok(cx);
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .on_action(cx.listener(|this, a: &PickType, _, cx| {
                this.driver = a.driver.to_string();
                cx.notify();
            }))
            .child(heading(if self.id.is_empty() {
                t!("snippet.new").to_string()
            } else {
                t!("snippet.edit").to_string()
            }))
            .child(
                FormRow::new(
                    t!("snippet.name").to_string(),
                    Input::new(&self.name).id("snippet-name").small(),
                )
                .hug_label(),
            )
            .child(
                FormRow::new(
                    t!("snippet.type").to_string(),
                    Button::new("snippet-type")
                        .small()
                        .label(type_label(&self.driver))
                        .icon(lucide("chevron-down"))
                        .on_click(|e: &ClickEvent, window, cx| {
                            TYPES
                                .iter()
                                .fold(NativeMenu::new(), |m, (d, _)| {
                                    m.menu(
                                        type_label(d),
                                        Box::new(PickType {
                                            driver: (*d).into(),
                                        }),
                                    )
                                })
                                .show(e.position(), window, cx);
                        }),
                )
                .hug_label(),
            )
            .child(
                FormRow::new(
                    t!("snippet.tags").to_string(),
                    Input::new(&self.tags).small(),
                )
                .hug_label(),
            )
            .child(
                FormRow::new(
                    t!("snippet.note").to_string(),
                    Input::new(&self.note).small(),
                )
                .hug_label(),
            )
            .child(
                div()
                    .h(tokens::snippet::CODE_HEIGHT.rems())
                    .child(Editor::new(&self.code).size_full()),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("snippet-add-placeholder")
                            .ghost()
                            .small()
                            .icon(lucide("braces"))
                            .tooltip(t!("snippet.add_placeholder").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.rewrite_code(add_placeholder, window, cx)
                            })),
                    )
                    .child(
                        Button::new("snippet-remove-placeholder")
                            .ghost()
                            .small()
                            .icon(lucide("eraser"))
                            .tooltip(t!("snippet.remove_placeholder").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.rewrite_code(|code, _| remove_placeholders(code), window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("snippet-cancel")
                            .small()
                            .label(t!("snippet.cancel").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("snippet-save")
                            .small()
                            .primary()
                            .label(t!("snippet.save").to_string())
                            .disabled(!ok)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}
