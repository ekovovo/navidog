//! A multiline batch statement must not grow the single-line footer or displace its controls.

use gpui_kit::component::{Sizable as _, Theme, ThemeMode, button::Button, h_flex, v_flex};
use gpui_kit::test::{TestSupportExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, TestAppContext, Window, WindowBounds,
    WindowOptions, px, size,
};
use navidog_ui::{skin::truncated, tokens};

struct Footer {
    sql: SharedString,
}

impl Render for Footer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(
            h_flex()
                .id("footer")
                .w_96()
                .min_w_0()
                .flex_none()
                .h(tokens::viewer::BOTTOM_BAR_HEIGHT.rems())
                .text_size(tokens::font::SIZE_BODY.rems())
                .line_height(tokens::font::LINE_BODY.rems())
                .gap_2()
                .child(truncated("sql", self.sql.clone()).flex_1().test_support())
                .child(
                    Button::new("refresh")
                        .xsmall()
                        .label("Refresh")
                        .w_20()
                        .flex_none(),
                )
                .test_support(),
        )
    }
}

#[gpui_kit::test]
fn batch_sql_stays_on_one_line_beside_footer_controls(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_ui::theme::init(cx);
    });
    let (handle, footer): (_, Entity<Footer>) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    // raw-px: the test window leaves space around the constrained footer.
                    size(px(500.), px(160.)),
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|_| Footer {
                    sql: SharedString::default(),
                })
            },
        )
        .unwrap()
    });
    let long_sql = format!(
        "DELETE FROM \"客户记录\" WHERE \"备注\" = '{}';",
        "批量删除的数据🙂".repeat(80)
    );
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|cx| Theme::change(mode, None, cx));
        for sql in [
            "DELETE FROM t WHERE id = 1;\nDELETE FROM t WHERE id = 2;\nDELETE FROM t WHERE id = 3;",
            "DELETE FROM t WHERE id = 1;\r\nDELETE FROM t WHERE id = 2;\r\nDELETE FROM t WHERE id = 3;",
            long_sql.as_str(),
        ] {
            cx.update(|cx| {
                footer.update(cx, |footer, cx| {
                    footer.sql = sql.to_owned().into();
                    cx.notify();
                });
            });
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let bar = window.find("footer").bounds();
                let text = window.find("sql").bounds();
                let refresh = window.find("refresh").bounds();
                let rem = window.rem_size();
                assert_eq!(
                    f32::from(bar.size.height).round(),
                    f32::from(tokens::viewer::BOTTOM_BAR_HEIGHT.to_pixels(rem)).round(),
                    "{mode:?}: the footer keeps its fixed height",
                );
                assert!(text.size.width > gpui_kit::Pixels::ZERO);
                assert!(text.size.height > gpui_kit::Pixels::ZERO);
                assert!(
                    text.size.height <= tokens::font::LINE_BODY.to_pixels(rem),
                    "{mode:?}: batch SQL must occupy one line, got {text:?}",
                );
                assert!(text.left() >= bar.left() && text.right() <= refresh.left());
                assert!(text.top() >= bar.top() && text.bottom() <= bar.bottom());
                assert!(refresh.right() <= bar.right());
                assert!(refresh.top() >= bar.top() && refresh.bottom() <= bar.bottom());
            })
            .unwrap();
        }
    }
}
