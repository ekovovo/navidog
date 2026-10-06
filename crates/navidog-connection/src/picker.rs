//! Connection type picker for a new connection (11-connection.md §1.1).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{
    Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, img,
};
use navidog_db::Family;
use navidog_ui::assets::driver_icon;
use navidog_ui::skin::heading;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| Picker::new(window, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(tokens::conn_type::WIDTH.to_pixels(window.rem_size()))
            .close_button(false)
            .child(view.clone())
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    All,
    Relational,
    NoSql,
    Cloud,
}

struct Picker {
    category: Category,
    search: Entity<InputState>,
    selected: Option<SharedString>,
    /// New connection from a URI (F-CONN-016)
    uri: Entity<InputState>,
}

impl Picker {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("conn.search").to_string()));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        let uri =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("conn.uri_hint").to_string()));
        cx.observe(&uri, |_, _, cx| cx.notify()).detach();
        Self {
            category: Category::All,
            search,
            selected: Some("mysql".into()),
            uri,
        }
    }

    fn next_from_uri(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.uri.read(cx).value().to_string();
        let Some(uri) = navidog_core::uri::DatabaseUri::parse_any(&text) else {
            window.push_notification(t!("conn.uri_invalid").to_string(), cx);
            return;
        };
        window.close_dialog(cx);
        crate::form::open_from_uri(uri, window, cx);
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(driver) = self.selected.clone() else {
            return;
        };
        window.close_dialog(cx);
        crate::form::open_new(driver, window, cx);
    }
}

impl Render for Picker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let query = self.search.read(cx).value().to_lowercase();
        let category = self.category;
        let drivers: Vec<_> = crate::drivers(cx)
            .into_iter()
            .map(|d| d.info().clone())
            .filter(|i| match category {
                Category::All => true,
                Category::Relational => i.family == Family::Relational,
                Category::NoSql => i.family != Family::Relational,
                Category::Cloud => false,
            })
            .filter(|i| query.is_empty() || i.name.to_lowercase().contains(&query))
            .collect();

        let categories = [
            (Category::All, t!("conn.category_all")),
            (Category::Relational, t!("conn.category_relational")),
            (Category::NoSql, t!("conn.category_nosql")),
            (Category::Cloud, t!("conn.category_cloud")),
        ];
        let sidebar = v_flex()
            .w(tokens::conn_type::SIDEBAR_WIDTH.rems())
            .flex_none()
            .py_2()
            .border_r_1()
            .border_color(p.separator)
            .children(categories.into_iter().map(|(c, label)| {
                let active = c == category;
                div()
                    .id(SharedString::from(format!("cat-{}", label)))
                    .px_3()
                    .h(tokens::list::ROW_HEIGHT.rems())
                    .flex()
                    .items_center()
                    .rounded_md()
                    .mx_2()
                    .when(active, |d| d.bg(p.selection).text_color(p.selection_text))
                    .child(label.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.category = c;
                        cx.notify();
                    }))
            }));

        let grid = h_flex()
            .flex_wrap()
            .gap_2()
            .p_2()
            .children(drivers.into_iter().map(|info| {
                let id = info.id.clone();
                let selected = self.selected.as_ref() == Some(&id);
                v_flex()
                    .id(SharedString::from(format!("driver-{id}")))
                    .w(tokens::conn_type::TILE_SIZE.rems())
                    .h(tokens::conn_type::TILE_SIZE.rems())
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .rounded_md()
                    .when(selected, |d| d.bg(p.selection_inactive))
                    .child(img(driver_icon(&id)).size(tokens::conn_type::TILE_ICON.rems()))
                    .child(info.name.clone())
                    .on_click(
                        cx.listener(move |this, e: &gpui_kit::ClickEvent, window, cx| {
                            this.selected = Some(id.clone());
                            if e.click_count() >= 2 {
                                this.next(window, cx);
                            }
                            cx.notify();
                        }),
                    )
            }));

        let main = v_flex()
            .flex_1()
            .min_w_0()
            .child(
                h_flex()
                    .h(tokens::conn_type::HEADER_HEIGHT.rems())
                    .justify_center()
                    .child(heading(t!("conn.new_connection").to_string())),
            )
            .child(
                h_flex()
                    .h(tokens::conn_type::SEARCH_ROW_HEIGHT.rems())
                    .px_4()
                    .justify_between()
                    .child(heading(t!("conn.select_type").to_string()))
                    .child(
                        Input::new(&self.search)
                            .prefix(gpui_kit::component::Icon::new(IconName::Search).small())
                            .w(tokens::conn_type::SEARCH_WIDTH.rems()),
                    ),
            )
            .child(div().flex_1().child(grid));

        v_flex()
            .h(tokens::conn_type::HEIGHT.rems())
            .child(
                h_flex()
                    .flex_1()
                    .items_stretch()
                    .min_h_0()
                    .child(sidebar)
                    .child(main),
            )
            .child(
                h_flex()
                    .h(tokens::conn_type::FOOTER_HEIGHT.rems())
                    .justify_between()
                    .border_t_1()
                    .border_color(p.separator)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&self.uri).w(tokens::conn_type::SEARCH_WIDTH.rems()))
                            .child(
                                Button::new("uri")
                                    .label(t!("conn.new_from_uri").to_string())
                                    .disabled(self.uri.read(cx).value().trim().is_empty())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.next_from_uri(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("cancel")
                                    .label(t!("conn.cancel").to_string())
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("next")
                                    .primary()
                                    .label(t!("conn.next").to_string())
                                    .disabled(self.selected.is_none())
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.next(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}
