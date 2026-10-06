//! 连接组的新建与重命名（F-CONN-013）。

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{App, AppContext as _, ParentElement as _, Window};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_ui::tokens;
use rust_i18n::t;

pub(crate) fn new_group(window: &mut Window, cx: &mut App) {
    let parent = match Workspace::global(cx).read(cx).selection() {
        Selection::Group(g) => Some(g.clone()),
        _ => None,
    };
    prompt_name(
        t!("nav.new_group_default").to_string(),
        window,
        cx,
        move |name, cx| {
            let store = ConnectionStore::global(cx);
            let id = store.update(cx, |s, cx| s.add_group(name, parent.clone(), cx));
            Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Group(id), cx));
        },
    );
}

pub(crate) fn rename_group(window: &mut Window, cx: &mut App) {
    let Selection::Group(id) = Workspace::global(cx).read(cx).selection().clone() else {
        return;
    };
    let Some(current) = ConnectionStore::global(cx)
        .read(cx)
        .groups()
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.name.clone())
    else {
        return;
    };
    prompt_name(current, window, cx, move |name, cx| {
        ConnectionStore::global(cx).update(cx, |s, cx| s.rename_group(&id, name, cx));
    });
}

/// 组名输入对话框。
// 偏差：Navicat 在树中原地编辑组名，这里先用对话框代替
fn prompt_name(
    initial: String,
    window: &mut Window,
    cx: &mut App,
    on_ok: impl Fn(String, &mut App) + 'static,
) {
    let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
    input.update(cx, |s, cx| s.focus(window, cx));
    let on_ok = std::rc::Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, window, _| {
        let commit = {
            let input = input.clone();
            let on_ok = on_ok.clone();
            move |cx: &mut App| {
                let name = input.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return false;
                }
                on_ok(name, cx);
                true
            }
        };
        let on_button = commit.clone();
        dialog
            .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
            .title(t!("nav.group_name").to_string())
            .child(Input::new(&input))
            .on_ok(move |_, _, cx| commit(cx))
            .footer(crate::secrets::footer(
                t!("conn.ok").to_string(),
                move |_, window, cx| {
                    if on_button(cx) {
                        window.close_dialog(cx);
                    }
                },
            ))
    });
}
