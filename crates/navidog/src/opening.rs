//! Dialogs that open or bookmark objects: Add to Favorites (F-APP-011) and Open navidog URI
//! (F-APP-014), with the mapping from a URI to one of the saved connections.

use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{App, AppContext as _, Entity, ParentElement as _, Styled as _, Window};
use navidog_core::connection::Connection;
use navidog_core::favorites::Favorites;
use navidog_core::object::{ObjectRef, Scope};
use navidog_core::store::ConnectionStore;
use navidog_core::uri::ObjectUri;
use navidog_core::workspace::Selection;
use navidog_ui::skin::FormRow;
use navidog_ui::tokens;
use rust_i18n::t;

/// What OK does; `true` closes the dialog. Enter in a field reaches the dialog's Confirm, so the
/// same closure goes to `on_ok` and to the footer's OK button.
type Ok = Rc<dyn Fn(&mut Window, &mut App) -> bool>;

fn confirm_on_enter(dialog: Dialog, ok: &Ok) -> Dialog {
    let ok = ok.clone();
    dialog.on_ok(move |_, window, cx| ok(window, cx))
}

fn footer(ok: Ok) -> impl gpui_kit::IntoElement {
    h_flex()
        .justify_end()
        .gap_2()
        .child(
            Button::new("cancel")
                .label(t!("dialog.cancel").to_string())
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(
            Button::new("ok")
                .primary()
                .label(t!("dialog.ok").to_string())
                .on_click(move |_, window, cx| {
                    if ok(window, cx) {
                        window.close_dialog(cx);
                    }
                }),
        )
}

fn input(window: &mut Window, cx: &mut App, value: String) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).default_value(value))
}

/// Asks for the favorite's name and optional ID (1–9) and adds it.
pub(crate) fn add_favorite(object: ObjectRef, window: &mut Window, cx: &mut App) {
    let name = input(window, cx, object.name.clone());
    let id = input(window, cx, String::new());
    let focus = name.clone();
    let (name_field, id_field) = (name.clone(), id.clone());
    let ok: Ok = Rc::new(move |_, cx| {
        let label = name.read(cx).value().trim().to_string();
        if label.is_empty() {
            return false;
        }
        let id = id
            .read(cx)
            .value()
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|n| (1..=9).contains(n));
        let object = object.clone();
        Favorites::global(cx).update(cx, |f, cx| f.add(object, label, id, cx));
        true
    });
    window.open_dialog(cx, move |dialog, window, _| {
        confirm_on_enter(dialog, &ok)
            .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
            .title(t!("fav.add_title").to_string())
            .child(
                v_flex()
                    .gap(tokens::form::ROW_GAP.rems())
                    .w_full()
                    .min_w_0()
                    .child(
                        FormRow::new(
                            t!("fav.name").to_string(),
                            Input::new(&name_field).w_full().min_w_0(),
                        )
                        .hug_label(),
                    )
                    .child(
                        FormRow::new(
                            t!("fav.id").to_string(),
                            Input::new(&id_field).w(tokens::form::FIELD_WIDTH_SHORT.rems()),
                        )
                        .hug_label(),
                    ),
            )
            .footer(footer(ok.clone()))
    });
    window.defer(cx, move |window, cx| {
        focus.update(cx, |state, cx| state.focus(window, cx));
    });
}

/// Asks for a navidog URI; `open` receives what it resolved to.
pub(crate) fn open_uri(
    window: &mut Window,
    cx: &mut App,
    open: impl Fn(Selection, &mut Window, &mut App) + 'static,
) {
    let field = input(window, cx, String::new());
    let input_field = field.clone();
    let ok: Ok = Rc::new(move |window, cx| {
        let text = field.read(cx).value().to_string();
        let resolved = ObjectUri::parse(&text)
            .ok_or_else(|| t!("uri.invalid").to_string())
            .and_then(|uri| {
                resolve(&uri, cx)
                    .ok_or_else(|| t!("uri.no_connection", name = uri.conn).to_string())
            });
        match resolved {
            Ok(sel) => {
                open(sel, window, cx);
                true
            }
            Err(message) => {
                window.push_notification(message, cx);
                false
            }
        }
    });
    let focus = input_field.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        confirm_on_enter(dialog, &ok)
            .w(tokens::form::FIELD_WIDTH_WIDE.to_pixels(window.rem_size()))
            .title(t!("uri.open_title").to_string())
            .child(Input::new(&input_field))
            .footer(footer(ok.clone()))
    });
    // `open_dialog` focuses the dialog itself, so a focus call before that does not stick
    window.defer(cx, move |window, cx| {
        focus.update(cx, |s, cx| s.focus(window, cx));
    });
}

/// Maps a URI to a saved connection (by name, then by host and port) and to what it names in it.
pub(crate) fn resolve(uri: &ObjectUri, cx: &App) -> Option<Selection> {
    let store = ConnectionStore::global(cx).read(cx);
    let conns = store.connections();
    let by_host = |c: &&Connection| {
        uri.host.as_ref().is_some_and(|h| &c.general.host == h)
            && uri.port.is_none_or(|p| c.general.port == Some(p))
    };
    let conn = conns
        .iter()
        .find(|c| c.name == uri.conn)
        .or_else(|| conns.iter().find(by_host))?;
    Some(match (&uri.database, uri.kind, &uri.name) {
        (Some(db), Some(kind), Some(name)) => Selection::Object(ObjectRef {
            scope: Scope::new(conn.id.clone(), db.clone(), uri.schema.clone()),
            kind,
            name: name.clone(),
        }),
        (Some(db), _, _) => Selection::Database(conn.id.clone(), db.clone()),
        _ => Selection::Connection(conn.id.clone()),
    })
}
