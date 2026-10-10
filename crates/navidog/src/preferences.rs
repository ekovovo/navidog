//! Preferences window (10-shell.md §10): general, tabs, records and advanced pages.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, WindowBounds, WindowOptions, div,
};
use navidog_core::object::ObjectRef;
use navidog_core::settings::{Appearance, Language, RowShade, Settings};
use navidog_core::store::ConnectionStore;
use navidog_ui::assets::lucide;
use navidog_ui::skin::{FormRow, confirm_with, form_note, heading, truncated};
use navidog_ui::{palette, tokens};
use rust_i18n::t;

struct PreferencesWindow(AnyWindowHandle);
impl Global for PreferencesWindow {}

/// Opens the preferences (a single window).
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<PreferencesWindow>().map(|w| w.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let bounds = WindowBounds::Windowed(gpui_kit::Bounds::centered(
        None,
        tokens::preferences_window_size(),
        cx,
    ));
    let options = WindowOptions {
        window_bounds: Some(bounds),
        ..TitleBar::window_options()
    };
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Preferences::new(window, cx))
    }) {
        cx.set_global(PreferencesWindow(handle));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    General,
    Tabs,
    Records,
    Advanced,
}

struct Preferences {
    page: Page,
    records_per_page: Entity<InputState>,
    grid_font_pt: Entity<InputState>,
}

impl Preferences {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify())
            .detach();
        // Both fields apply on Return or when they lose focus, not per keystroke: typing "100"
        // would otherwise save the "10" on the way. A bad value goes back to the saved one.
        let per_page = Settings::global(cx).records_per_page.to_string();
        let records_per_page = cx.new(|cx| InputState::new(window, cx).default_value(per_page));
        cx.subscribe_in(
            &records_per_page,
            window,
            |_, input, e: &InputEvent, window, cx| {
                if !matches!(e, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    return;
                }
                match input.read(cx).value().trim().parse::<u64>() {
                    Ok(n) if n > 0 => Settings::update(cx, |s| s.records_per_page = n),
                    _ => {
                        let saved = Settings::global(cx).records_per_page.to_string();
                        input.update(cx, |s, cx| s.set_value(saved, window, cx));
                    }
                }
            },
        )
        .detach();
        let font = Settings::global(cx).grid_font_pt.to_string();
        let grid_font_pt = cx.new(|cx| InputState::new(window, cx).default_value(font));
        cx.subscribe_in(
            &grid_font_pt,
            window,
            |_, input, e: &InputEvent, window, cx| {
                if !matches!(e, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    return;
                }
                match input.read(cx).value().trim().parse::<f32>() {
                    Ok(n) if Settings::GRID_FONT_PT.contains(&n) => {
                        Settings::update(cx, |s| s.grid_font_pt = n);
                        cx.refresh_windows();
                    }
                    _ => {
                        let saved = Settings::global(cx).grid_font_pt.to_string();
                        input.update(cx, |s, cx| s.set_value(saved, window, cx));
                    }
                }
            },
        )
        .detach();
        Self {
            page: Page::General,
            records_per_page,
            grid_font_pt,
        }
    }

    /// A Navicat-style section label ("Main Window:").
    fn section(title: impl Into<SharedString>) -> impl IntoElement {
        heading(SharedString::from(format!("{}:", title.into())))
    }

    /// One setting on a line: the label, then the control. Appearance and language
    /// are popups in Navicat; a horizontal radio group is the same row without a
    /// menu that would keep stale labels after a language change.
    fn setting_row(label: impl Into<SharedString>, control: impl IntoElement) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .min_w_0()
            .child(label.into())
            .child(control)
    }

    /// Tabs (F-PREF-003): where editors open, and the startup screen.
    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Settings::global(cx).clone();
        let p = palette(cx);
        let startup: Vec<ObjectRef> = s
            .startup_tabs
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(Self::section(t!("prefs.open_tabs").to_string()))
            .child(
                Checkbox::new("open-in-new-window")
                    .label(t!("prefs.open_in_new_window").to_string())
                    .checked(s.open_in_new_window)
                    .on_click(|v: &bool, _, cx| {
                        let v = *v;
                        Settings::update(cx, |s| s.open_in_new_window = v);
                        cx.refresh_windows();
                    }),
            )
            .child(Self::section(t!("prefs.startup_screen").to_string()))
            .child(form_note(t!("prefs.startup_hint").to_string(), cx))
            .children(startup.into_iter().enumerate().map(|(ix, o)| {
                h_flex()
                    .gap_2()
                    .min_w_0()
                    .child(
                        truncated(("startup-tab", ix), o.name.clone())
                            .flex_1()
                            .text_color(p.label),
                    )
                    .child(
                        Button::new(("startup-remove", ix))
                            .ghost()
                            .xsmall()
                            .icon(lucide("minus"))
                            .on_click(move |_, _, cx| {
                                Settings::update(cx, |s| {
                                    if ix < s.startup_tabs.len() {
                                        s.startup_tabs.remove(ix);
                                    }
                                });
                                cx.refresh_windows();
                            }),
                    )
            }))
    }

    /// Records (F-PREF-005): "Limit records [n] records per page" on one line.
    fn render_records(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Settings::global(cx).clone();
        let shades = [RowShade::None, RowShade::Alternate, RowShade::EveryThird];
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(Self::section(t!("prefs.records").to_string()))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new("limit-records")
                            .label(t!("prefs.limit_records").to_string())
                            .checked(s.limit_records)
                            .on_click(|v: &bool, _, cx| {
                                let v = *v;
                                Settings::update(cx, |s| s.limit_records = v);
                                cx.refresh_windows();
                            }),
                    )
                    .child(
                        Input::new(&self.records_per_page)
                            .w(tokens::form::FIELD_WIDTH_SHORT.rems())
                            .disabled(!s.limit_records),
                    )
                    .child(t!("prefs.records_per_page").to_string()),
            )
            .child(
                Checkbox::new("auto-begin-transaction")
                    .label(t!("prefs.auto_begin_transaction").to_string())
                    .checked(s.auto_begin_transaction)
                    .on_click(|v: &bool, _, cx| {
                        let v = *v;
                        Settings::update(cx, |s| s.auto_begin_transaction = v);
                        cx.refresh_windows();
                    }),
            )
            .child(
                Checkbox::new("sync-record")
                    .label(t!("prefs.sync_record").to_string())
                    .checked(s.sync_record)
                    .on_click(|v: &bool, _, cx| {
                        let v = *v;
                        Settings::update(cx, |s| s.sync_record = v);
                    }),
            )
            .child(Self::section(t!("prefs.grid").to_string()))
            .child(
                Checkbox::new("click-to-edit")
                    .label(t!("prefs.click_to_edit").to_string())
                    .checked(s.click_to_edit)
                    .on_click(|v: &bool, _, cx| {
                        let v = *v;
                        Settings::update(cx, |s| s.click_to_edit = v);
                    }),
            )
            .child(
                Checkbox::new("use-default-grid-font")
                    .label(t!("prefs.use_default_font").to_string())
                    .checked(s.use_default_grid_font)
                    .on_click(|v: &bool, _, cx| {
                        let v = *v;
                        Settings::update(cx, |s| s.use_default_grid_font = v);
                        cx.refresh_windows();
                    }),
            )
            .child(Self::setting_row(
                format!("{}:", t!("prefs.font_size")),
                Input::new(&self.grid_font_pt)
                    .w(tokens::form::FIELD_WIDTH_SHORT.rems())
                    .disabled(s.use_default_grid_font),
            ))
            .child(Self::setting_row(
                format!("{}:", t!("prefs.row_shading")),
                RadioGroup::horizontal("row-shade")
                    .selected_index(shades.iter().position(|a| *a == s.row_shade))
                    .child(Radio::new("shade-none").label(t!("prefs.shade_none").to_string()))
                    .child(
                        Radio::new("shade-alternate")
                            .label(t!("prefs.shade_alternate").to_string()),
                    )
                    .child(
                        Radio::new("shade-every-third")
                            .label(t!("prefs.shade_every_third").to_string()),
                    )
                    .on_click(move |ix: &usize, _, cx| {
                        let shade = shades[*ix];
                        Settings::update(cx, |s| s.row_shade = shade);
                        cx.refresh_windows();
                    }),
            ))
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Settings::global(cx).clone();
        let appearances = [Appearance::System, Appearance::Light, Appearance::Dark];
        let languages = [Language::System, Language::ZhCn, Language::En];
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(Self::setting_row(
                format!("{}:", t!("prefs.appearance")),
                RadioGroup::horizontal("appearance")
                    .selected_index(appearances.iter().position(|a| *a == s.appearance))
                    .child(Radio::new("system").label(t!("prefs.appearance_system").to_string()))
                    .child(Radio::new("light").label(t!("prefs.appearance_light").to_string()))
                    .child(Radio::new("dark").label(t!("prefs.appearance_dark").to_string()))
                    .on_click(move |ix: &usize, window, cx| {
                        let appearance = appearances[*ix];
                        Settings::update(cx, |s| s.appearance = appearance);
                        navidog_ui::theme::apply_appearance(appearance.is_dark(), Some(window), cx);
                    }),
            ))
            .child(Self::setting_row(
                format!("{}:", t!("prefs.language")),
                RadioGroup::horizontal("language")
                    .selected_index(languages.iter().position(|l| *l == s.language))
                    .child(Radio::new("lang-system").label(t!("prefs.language_system").to_string()))
                    .child(Radio::new("lang-zh").label("简体中文"))
                    .child(Radio::new("lang-en").label("English"))
                    .on_click(move |ix: &usize, _, cx| {
                        let language = languages[*ix];
                        Settings::update(cx, |s| s.language = language);
                        gpui_kit::component::set_locale(language.locale());
                        crate::menus::set_menus(cx);
                        cx.refresh_windows();
                    }),
            ))
            .child(Self::section(t!("prefs.main_window").to_string()))
            .child(
                Checkbox::new("show-schema-objects")
                    .label(t!("prefs.show_schema_objects").to_string())
                    .checked(s.show_schema_objects)
                    .on_click(|v: &bool, _, cx| {
                        let v = *v;
                        Settings::update(cx, |s| s.show_schema_objects = v);
                        cx.refresh_windows();
                    }),
            )
            .child(
                Checkbox::new("show-table-children")
                    .label(t!("prefs.show_table_children").to_string())
                    .checked(s.show_table_children)
                    .on_click(|v: &bool, _, cx| {
                        let v = *v;
                        Settings::update(cx, |s| s.show_table_children = v);
                        cx.refresh_windows();
                    }),
            )
            .child(
                Checkbox::new("auto-connect")
                    .label(t!("prefs.auto_connect").to_string())
                    .checked(s.auto_connect_on_startup)
                    .on_click(|v: &bool, _, cx| {
                        let v = *v;
                        Settings::update(cx, |s| s.auto_connect_on_startup = v);
                        cx.refresh_windows();
                    }),
            )
    }

    fn render_advanced(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let dir = navidog_core::paths::config_dir().display().to_string();
        let is_master = ConnectionStore::global(cx).read(cx).is_master();
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(Self::section(t!("prefs.settings_location").to_string()))
            .child(
                truncated("settings-location", dir)
                    .w_full()
                    .text_color(p.label_secondary),
            )
            .child(
                Button::new("reveal")
                    .label(t!("prefs.reveal").to_string())
                    .on_click(|_, _, cx| {
                        cx.reveal_path(&navidog_core::paths::config_dir());
                    }),
            )
            .child(Self::section(t!("prefs.master_password").to_string()))
            .child(div().text_color(p.label_secondary).child(if is_master {
                t!("prefs.master_enabled").to_string()
            } else {
                t!("prefs.master_disabled").to_string()
            }))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("master-set")
                            .label(
                                if is_master {
                                    t!("prefs.master_change")
                                } else {
                                    t!("prefs.master_set")
                                }
                                .to_string(),
                            )
                            .on_click(|_, window, cx| open_master_dialog(window, cx)),
                    )
                    .when(is_master, |r| {
                        r.child(
                            Button::new("master-clear")
                                .label(t!("prefs.master_clear").to_string())
                                .on_click(|_, window, cx| {
                                    let store = ConnectionStore::global(cx);
                                    match store.update(cx, |s, cx| s.set_master_password(None, cx))
                                    {
                                        Ok(dropped) => notify_dropped(&dropped, window, cx),
                                        Err(e) => {
                                            let message =
                                                navidog_connection::store_error_message(&e);
                                            window.push_notification(message, cx);
                                        }
                                    }
                                }),
                        )
                    }),
            )
    }
}

impl Render for Preferences {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let pages = [
            (Page::General, t!("prefs.general")),
            (Page::Tabs, t!("prefs.tabs")),
            (Page::Records, t!("prefs.records")),
            (Page::Advanced, t!("prefs.advanced")),
        ];
        let page = self.page;
        let content = match page {
            Page::General => self.render_general(cx).into_any_element(),
            Page::Tabs => self.render_tabs(cx).into_any_element(),
            Page::Records => self.render_records(cx).into_any_element(),
            Page::Advanced => self.render_advanced(cx).into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(p.window_bg)
            .text_color(p.label)
            .child(
                TitleBar::new().child(
                    div()
                        .flex_1()
                        .flex()
                        .justify_center()
                        .child(t!("prefs.title").to_string()),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(
                        v_flex()
                            .w(tokens::preferences::NAV_WIDTH.rems())
                            .flex_none()
                            .py_2()
                            .border_r_1()
                            .border_color(p.separator)
                            .children(pages.into_iter().map(|(pg, label)| {
                                div()
                                    .id(SharedString::from(format!("page-{label}")))
                                    .mx_2()
                                    .px_3()
                                    .h(tokens::list::ROW_HEIGHT.rems())
                                    .flex()
                                    .items_center()
                                    .rounded_md()
                                    .when(pg == page, |d| {
                                        d.bg(p.selection).text_color(p.selection_text)
                                    })
                                    .child(label.to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.page = pg;
                                        cx.notify();
                                    }))
                            })),
                    )
                    .child(
                        v_flex()
                            .id("prefs-page")
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p(tokens::form::MARGIN.rems())
                            .child(div().min_w_0().w_full().child(content)),
                    ),
            )
    }
}

/// Sets or changes the master password: the new password is entered twice.
pub fn open_master_dialog(window: &mut Window, cx: &mut App) {
    let first = cx.new(|cx| InputState::new(window, cx).masked(true));
    let second = cx.new(|cx| InputState::new(window, cx).masked(true));
    let view = cx.new(|_| MasterDialog {
        first: first.clone(),
        second,
        error: None,
    });
    window.open_dialog(cx, move |dialog, window, _| {
        confirm_with(dialog, &view, MasterDialog::confirm)
            .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
            .close_button(false)
            .child(view.clone())
    });
    window.defer(cx, move |window, cx| {
        first.update(cx, |state, cx| state.focus(window, cx));
    });
}

struct MasterDialog {
    first: Entity<InputState>,
    second: Entity<InputState>,
    error: Option<SharedString>,
}

impl MasterDialog {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let a = self.first.read(cx).value().to_string();
        let b = self.second.read(cx).value().to_string();
        if a.is_empty() || a != b {
            self.error = Some(t!("prefs.master_mismatch").to_string().into());
            cx.notify();
            return;
        }
        let store = ConnectionStore::global(cx);
        match store.update(cx, |s, cx| s.set_master_password(Some(&a), cx)) {
            Ok(dropped) => {
                window.close_dialog(cx);
                notify_dropped(&dropped, window, cx);
            }
            Err(e) => {
                self.error = Some(navidog_connection::store_error_message(&e).into());
                cx.notify();
            }
        }
    }
}

/// Saved passwords no key can open any more were dropped while re-encrypting; say whose.
fn notify_dropped(connections: &[String], window: &mut Window, cx: &mut App) {
    if connections.is_empty() {
        return;
    }
    let names = connections.join(", ");
    window.push_notification(t!("prefs.master_dropped", names = names).to_string(), cx);
}

impl Render for MasterDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .w_full()
            .min_w_0()
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(t!("prefs.master_password").to_string()))
            .child(
                FormRow::new(
                    t!("prefs.master_new").to_string(),
                    Input::new(&self.first).w_full().min_w_0(),
                )
                .hug_label(),
            )
            .child(
                FormRow::new(
                    t!("prefs.master_confirm").to_string(),
                    Input::new(&self.second).w_full().min_w_0(),
                )
                .hug_label(),
            )
            .when_some(self.error.clone(), |v, e| {
                v.child(FormRow::bare(div().text_color(p.danger).child(e)).hug_label())
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel")
                            .label(t!("prefs.cancel").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("ok")
                            .primary()
                            .label(t!("prefs.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    ),
            )
    }
}
