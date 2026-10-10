//! Connection profiles (F-CONN-021): named settings of one connection, e.g. for another user.
//! The active profile is what the form edits and what opening connects with.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window,
};
use navidog_core::store::ConnectionStore;
use navidog_db::sessions::Sessions;
use navidog_ui::skin::{FormRow, confirm_with, heading};
use navidog_ui::tokens;
use rust_i18n::t;

use crate::secrets::footer;

/// Display name of a profile ("" is the default one).
pub fn label(name: &str) -> String {
    if name.is_empty() {
        t!("conn.profile_default").to_string()
    } else {
        name.to_string()
    }
}

/// Applies `f` to the connection and reconnects it if it was open, so the session uses the new settings.
fn change(
    id: &str,
    window: &mut Window,
    cx: &mut App,
    f: impl FnOnce(&mut navidog_core::connection::Connection),
) {
    let open = Sessions::global(cx).read(cx).state(id).is_open();
    ConnectionStore::global(cx).update(cx, |s, cx| s.update(id, f, cx));
    if open {
        Sessions::global(cx).update(cx, |s, cx| s.close(id, cx));
        crate::open_connection(id.to_string(), window, cx);
    }
}

pub fn switch(id: &str, name: &str, window: &mut Window, cx: &mut App) {
    let name = name.to_string();
    change(id, window, cx, |c| {
        c.switch_profile(&name);
    });
}

pub fn delete(id: String, window: &mut Window, cx: &mut App) {
    let Some(name) = ConnectionStore::global(cx)
        .read(cx)
        .get(&id)
        .filter(|c| !c.profiles.is_empty())
        .map(|c| label(&c.profile))
    else {
        return;
    };
    window.open_alert_dialog(cx, move |alert, _, _| {
        let id = id.clone();
        alert
            .icon(navidog_ui::assets::lucide("triangle-alert"))
            .title(SharedString::from(
                t!("conn.confirm_delete_profile", name = name).to_string(),
            ))
            .show_cancel(true)
            .ok_text(t!("conn.delete").to_string())
            .cancel_text(t!("conn.cancel").to_string())
            .on_ok(move |_, window, cx| {
                change(&id, window, cx, |c| {
                    c.remove_active_profile();
                });
                true
            })
    });
}

/// Asks for a name, adds the profile as a copy of the active settings and opens the form on it.
pub fn new(id: String, window: &mut Window, cx: &mut App) {
    let input = cx.new(|cx| InputState::new(window, cx));
    let view = cx.new(|_| NewProfile {
        id,
        input: input.clone(),
    });
    window.open_dialog(cx, move |dialog, window, _| {
        confirm_with(dialog, &view, NewProfile::confirm)
            .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
            .close_button(false)
            .child(view.clone())
    });
    window.defer(cx, move |window, cx| {
        input.update(cx, |state, cx| state.focus(window, cx));
    });
}

struct NewProfile {
    id: String,
    input: Entity<InputState>,
}

impl NewProfile {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).value().trim().to_string();
        let taken = ConnectionStore::global(cx)
            .read(cx)
            .get(&self.id)
            .is_some_and(|c| c.profile_names().any(|n| label(n) == name));
        if name.is_empty() || taken {
            return;
        }
        window.close_dialog(cx);
        let id = self.id.clone();
        change(&id, window, cx, |c| c.add_profile(name));
        crate::form::open_edit(id, window, cx);
    }
}

impl Render for NewProfile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .min_w_0()
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(t!("conn.new_profile").to_string()))
            .child(
                FormRow::new(
                    t!("conn.profile_name").to_string(),
                    Input::new(&self.input)
                        .id("profile-name")
                        .w_full()
                        .min_w_0(),
                )
                .hug_label(),
            )
            .child(footer(
                t!("conn.ok").to_string(),
                cx.listener(|this, _, window, cx| this.confirm(window, cx)),
            ))
    }
}
