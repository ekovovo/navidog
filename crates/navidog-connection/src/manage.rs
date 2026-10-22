//! Manage connections window (F-CONN-015, ManageConnectionViewController.nib): every connection
//! in one table, with new, edit, star, hide, delete and a search that highlights matches.
//! Changes go through the shared connection commands, so the navigator updates at once.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Disableable as _, IconName, Sizable as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window,
    WindowBounds, WindowOptions, div,
};
use navidog_core::connection::Connection;
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_ui::assets::lucide;
use navidog_ui::skin::highlighted;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

struct ManageWindow(AnyWindowHandle);
impl Global for ManageWindow {}

/// Star column plus six fixed columns, plus the row padding. `px_3` is 0.75rem on each side
/// (ListItem, and the header which matches it). The scrollport is the window, so this wider
/// table is what scrolls sideways.
fn table_width() -> gpui_kit::Rems {
    let pt = tokens::list::ROW_HEIGHT.0
        + tokens::manage_conn::COLUMN_WIDTH.0 * 6.
        + 0.75 * tokens::BASE_PT * 2.;
    tokens::Pt(pt).rems()
}

pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<ManageWindow>().map(|w| w.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let size = tokens::window_size(tokens::manage_conn::WIDTH, tokens::manage_conn::HEIGHT);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds::centered(
            None, size, cx,
        ))),
        ..TitleBar::window_options()
    };
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Manage::new(window, cx))
    }) {
        cx.set_global(ManageWindow(handle));
    }
}

pub(crate) struct Manage {
    search: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl Manage {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("conn.manage_search").to_string())
        });
        let _subs = vec![
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&Workspace::global(cx), |_, _, cx| cx.notify()),
        ];
        Self { search, _subs }
    }
}

/// The columns, in order, and the text each shows for a connection.
fn cells(c: &Connection, group: Option<&str>, cx: &App) -> [String; 6] {
    [
        c.name.clone(),
        crate::driver_name(&c.driver, cx).to_string(),
        c.general
            .file
            .clone()
            .unwrap_or_else(|| c.general.host.clone()),
        c.general.port.map(|p| p.to_string()).unwrap_or_default(),
        c.general.user.clone(),
        group.unwrap_or_default().to_string(),
    ]
}

impl Render for Manage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let query = self.search.read(cx).value().trim().to_string();
        let selected = crate::selected_connections(cx);
        let store = ConnectionStore::global(cx).read(cx);
        let rows: Vec<_> = store
            .connections()
            .iter()
            .map(|c| {
                let group = c
                    .group
                    .as_ref()
                    .and_then(|g| store.groups().iter().find(|x| &x.id == g));
                (
                    c.id.clone(),
                    c.starred,
                    c.hidden,
                    cells(c, group.map(|g| g.name.as_str()), cx),
                )
            })
            .filter(|(.., cells)| {
                query.is_empty()
                    || cells
                        .iter()
                        .any(|s| !navidog_ui::skin::find_matches(s, &query).is_empty())
            })
            .collect();

        let column = || {
            div()
                .w(tokens::manage_conn::COLUMN_WIDTH.rems())
                .flex_none()
                .truncate()
                .px_2()
        };
        let headers = [
            t!("conn.name"),
            t!("conn.manage_type"),
            t!("conn.host"),
            t!("conn.port"),
            t!("conn.user"),
            t!("conn.manage_group"),
        ];
        let button = |id: &'static str, icon: &'static str, tip: String| {
            Button::new(id)
                .ghost()
                .small()
                .icon(lucide(icon))
                .tooltip(tip)
        };
        let none = selected.is_empty();

        let toolbar = h_flex()
            .h(tokens::manage_conn::BAR_HEIGHT.rems())
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(p.separator)
            .child(
                button("manage-new", "plus", t!("conn.new_connection").to_string())
                    .on_click(|_, window, cx| crate::picker::open(window, cx)),
            )
            .child(
                button("manage-edit", "pencil", t!("conn.manage_edit").to_string())
                    .disabled(selected.len() != 1)
                    .on_click(|_, window, cx| crate::edit_selected(window, cx)),
            )
            .child(
                button("manage-star", "star", t!("conn.manage_star").to_string())
                    .disabled(none)
                    .on_click(|_, window, cx| crate::toggle_star(window, cx)),
            )
            .child(
                button("manage-hide", "eye-off", t!("conn.manage_hide").to_string())
                    .disabled(none)
                    .on_click(|_, window, cx| crate::toggle_hidden(window, cx)),
            )
            .child(
                button("manage-delete", "trash", t!("conn.delete").to_string())
                    .disabled(selected.len() != 1)
                    .on_click(|_, window, cx| crate::delete_selected(window, cx)),
            )
            .child(div().flex_1())
            .child(
                Input::new(&self.search)
                    .id("manage-search")
                    .small()
                    .prefix(gpui_kit::component::Icon::new(IconName::Search).small())
                    .cleanable(true)
                    .w(tokens::manage_conn::SEARCH_WIDTH.rems()),
            );

        // Same inset as the ListItem rows (px_3) plus the star column
        let header = h_flex()
            .h(tokens::list::ROW_HEIGHT.rems())
            .px_3()
            .child(div().w(tokens::list::ROW_HEIGHT.rems()).flex_none())
            .border_b_1()
            .border_color(p.separator)
            .text_color(p.label_secondary)
            .children(headers.into_iter().map(|h| column().child(h.to_string())));

        let body = rows
            .into_iter()
            .enumerate()
            .map(|(ix, (id, starred, hidden, cells))| {
                let is_selected = selected.contains(&id);
                ListItem::new(("manage-conn", ix))
                    .selected(is_selected)
                    .on_click(move |e, window, cx| {
                        Workspace::global(cx)
                            .update(cx, |w, cx| w.select(Selection::Connection(id.clone()), cx));
                        if e.click_count() >= 2 {
                            crate::edit_selected(window, cx);
                        }
                    })
                    .child(
                        h_flex()
                            // Hidden connections are listed dimmed
                            .when(hidden, |d| d.text_color(p.label_tertiary))
                            .child(
                                div()
                                    .w(tokens::list::ROW_HEIGHT.rems())
                                    .flex_none()
                                    .flex()
                                    .justify_center()
                                    .when(starred, |d| d.child(lucide("star").small())),
                            )
                            .children(
                                cells
                                    .iter()
                                    .map(|s| column().child(highlighted(s.clone(), &query, cx))),
                            ),
                    )
            });

        crate::bind_actions(
            v_flex()
                .id("manage-connections")
                .size_full()
                .bg(p.window_bg)
                .text_color(p.label)
                .child(
                    TitleBar::new().child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_center()
                            .child(SharedString::from(t!("conn.manage_title").to_string())),
                    ),
                )
                .child(toolbar)
                .child(
                    div()
                        .id("manage-rows")
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_scrollbar()
                        .child(
                            v_flex()
                                .w(table_width())
                                .flex_none()
                                .child(header)
                                .children(body),
                        ),
                ),
        )
    }
}
