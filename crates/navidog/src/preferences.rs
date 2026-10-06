//! 偏好设置窗口（10-shell.md 第 10 节）：常规、高级。

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{TitleBar, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, WindowBounds, WindowOptions, div,
};
use navidog_core::settings::{Appearance, Language, Settings};
use navidog_core::store::ConnectionStore;
use navidog_ui::skin::{FormRow, heading};
use navidog_ui::{palette, tokens};
use rust_i18n::t;

struct PreferencesWindow(AnyWindowHandle);
impl Global for PreferencesWindow {}

/// 打开偏好设置（单例窗口）。
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
    Advanced,
}

struct Preferences {
    page: Page,
}

impl Preferences {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify())
            .detach();
        Self {
            page: Page::General,
        }
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Settings::global(cx).clone();
        let appearances = [Appearance::System, Appearance::Light, Appearance::Dark];
        let languages = [Language::System, Language::ZhCn, Language::En];
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(FormRow::new(
                t!("prefs.appearance").to_string(),
                RadioGroup::vertical("appearance")
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
            .child(FormRow::new(
                t!("prefs.language").to_string(),
                RadioGroup::vertical("language")
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
            .child(FormRow::bare(
                Checkbox::new("auto-connect")
                    .label(t!("prefs.auto_connect").to_string())
                    .checked(s.auto_connect_on_startup)
                    .on_click(|v: &bool, _, cx| {
                        let v = *v;
                        Settings::update(cx, |s| s.auto_connect_on_startup = v);
                        cx.refresh_windows();
                    }),
            ))
    }

    fn render_advanced(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let dir = navidog_core::paths::config_dir();
        let is_master = ConnectionStore::global(cx).read(cx).is_master();
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(FormRow::new(
                t!("prefs.settings_location").to_string(),
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_color(p.label_secondary)
                            .child(dir.display().to_string()),
                    )
                    .child(
                        Button::new("reveal")
                            .label(t!("prefs.reveal").to_string())
                            .on_click(move |_, _, cx| {
                                cx.reveal_path(&navidog_core::paths::config_dir());
                            }),
                    ),
            ))
            .child(FormRow::new(
                t!("prefs.master_password").to_string(),
                v_flex()
                    .gap_1()
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
                                            if let Err(e) = store
                                                .update(cx, |s, cx| s.set_master_password(None, cx))
                                            {
                                                window.push_notification(e.to_string(), cx);
                                            }
                                        }),
                                )
                            }),
                    ),
            ))
    }
}

impl Render for Preferences {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let pages = [
            (Page::General, t!("prefs.general")),
            (Page::Advanced, t!("prefs.advanced")),
        ];
        let page = self.page;
        let content = match page {
            Page::General => self.render_general(cx).into_any_element(),
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
                    .items_stretch()
                    .child(
                        v_flex()
                            .w(tokens::pane::NAVIGATOR_WIDTH.rems())
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
                            .flex_1()
                            .p(tokens::form::MARGIN.rems())
                            .child(content),
                    ),
            )
    }
}

/// 设置或修改主密码：两次输入新密码。
fn open_master_dialog(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| MasterDialog {
        first: cx.new(|cx| InputState::new(window, cx).masked(true)),
        second: cx.new(|cx| InputState::new(window, cx).masked(true)),
        error: None,
    });
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
            .close_button(false)
            .child(view.clone())
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
            Ok(()) => window.close_dialog(cx),
            Err(e) => {
                self.error = Some(e.to_string().into());
                cx.notify();
            }
        }
    }
}

impl Render for MasterDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(t!("prefs.master_password").to_string()))
            .child(FormRow::new(
                t!("prefs.master_new").to_string(),
                Input::new(&self.first).w(tokens::form::FIELD_WIDTH.rems()),
            ))
            .child(FormRow::new(
                t!("prefs.master_confirm").to_string(),
                Input::new(&self.second).w(tokens::form::FIELD_WIDTH.rems()),
            ))
            .when_some(self.error.clone(), |v, e| {
                v.child(FormRow::bare(div().text_color(p.danger).child(e)))
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
