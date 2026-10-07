//! Headless UI test of the navigator: ⌘-click multi-select, dragging connections into a group and
//! back to the top level, and editing group names in place.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, FocusHandle, InputEvent as _, InteractiveElement as _, IntoElement,
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement as _, Render, Styled as _,
    TestAppContext, Window, WindowBounds, WindowOptions, div, point, size,
};
use navidog_core::actions::{NewGroup, RenameGroup};
use navidog_core::connection::Connection;
use navidog_core::crypto::Key;
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_navigator::NavigatorPanel;

struct Host {
    focus: FocusHandle,
    navigator: gpui_kit::Entity<NavigatorPanel>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        navidog_navigator::bind_actions(
            div()
                .id("host")
                .track_focus(&self.focus)
                .size_full()
                .child(self.navigator.clone()),
        )
    }
}

fn cmd_click(window: &mut Window, id: &str, cx: &mut gpui_kit::App) {
    let position = window.find(id.to_string()).bounds().center();
    let modifiers = Modifiers::command();
    let down = MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers,
        click_count: 1,
        first_mouse: false,
    };
    window.dispatch_event(down.to_platform_input(), cx);
    let up = MouseUpEvent {
        button: MouseButton::Left,
        position,
        modifiers,
        click_count: 1,
    };
    window.dispatch_event(up.to_platform_input(), cx);
    window.render_frame(cx);
}

#[gpui_kit::test]
fn multi_select_drag_and_rename(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-nav-test-{}", std::process::id()));
    // SAFETY: this test binary has a single test; nothing else reads the environment concurrently
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
    let (group, a, b) = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_navigator::init(cx);
        ConnectionStore::global(cx).update(cx, |s, cx| {
            s.set_key(Key::random());
            let group = s.add_group("Group".into(), None, cx);
            let mut ids = ["A", "B"].map(|name| {
                let mut c = Connection::new("sqlite");
                c.name = name.into();
                let id = c.id.clone();
                s.upsert(c, &[], cx).unwrap();
                id
            });
            ids.sort_by_key(|id| s.get(id).unwrap().name.clone());
            let [a, b] = ids;
            (group, a, b)
        })
    });
    let (handle, host) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(400.), gpui_kit::px(600.));
        let bounds = gpui_kit::Bounds {
            origin: Default::default(),
            size: test_size,
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let navigator = NavigatorPanel::shared(window, cx);
            cx.new(|cx| Host {
                focus: cx.focus_handle(),
                navigator,
            })
        })
        .unwrap()
    });
    let (row_a, row_b, row_g) = (format!("c:{a}"), format!("c:{b}"), format!("g:{group}"));
    let selections = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            Workspace::global(cx)
                .read(cx)
                .selections()
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    let group_of = |cx: &mut TestAppContext, id: &str| {
        cx.update(|cx| {
            ConnectionStore::global(cx)
                .read(cx)
                .get(id)
                .unwrap()
                .group
                .clone()
        })
    };

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(row_a.clone(), cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| window.press("shift-down", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        selections(cx),
        [
            Selection::Connection(b.clone()),
            Selection::Connection(a.clone())
        ]
    );
    // A plain arrow replaces the range; B is the last row, so up returns to A alone
    cx.update_window(handle, |_, window, cx| window.press("up", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(selections(cx), [Selection::Connection(a.clone())]);

    cx.update_window(handle, |_, window, cx| {
        window.click(row_a.clone(), cx);
        cmd_click(window, &row_b, cx);
    })
    .unwrap();
    assert_eq!(
        selections(cx),
        [
            Selection::Connection(b.clone()),
            Selection::Connection(a.clone())
        ]
    );

    // Dragging one selected connection moves the whole selection into the group
    cx.update_window(handle, |_, window, cx| {
        window.drag_to(row_a.clone(), row_g.clone(), cx)
    })
    .unwrap();
    assert_eq!(
        (group_of(cx, &a), group_of(cx, &b)),
        (Some(group.clone()), Some(group.clone()))
    );

    // Expand the group, then drop onto the blank area below the rows: back to the top level
    cx.update_window(handle, |_, window, cx| {
        window.double_click(row_g.clone(), cx);
        window.render_frame(cx);
        window.click(row_a.clone(), cx);
        let from = window.find(row_a.clone()).bounds().center();
        // raw-px: a point in the blank area of the 600px-high test window
        window.drag(from, point(gpui_kit::px(100.), gpui_kit::px(500.)), cx);
    })
    .unwrap();
    assert_eq!(
        (group_of(cx, &a), group_of(cx, &b)),
        (None, Some(group.clone()))
    );

    let group_names = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let store = ConnectionStore::global(cx);
            let store = store.read(cx);
            store
                .groups()
                .iter()
                .map(|g| (g.name.clone(), g.parent.clone()))
                .collect::<Vec<_>>()
        })
    };
    // Commands run in their own update so their notifications are flushed before typing
    let command = |cx: &mut TestAppContext, action: Box<dyn gpui_kit::Action>| {
        cx.update_window(handle, |_, window, cx| window.dispatch_action(action, cx))
            .unwrap();
        cx.run_until_parked();
    };
    let type_then = |cx: &mut TestAppContext, text: &str, key: &str| {
        cx.update_window(handle, |_, window, cx| {
            window.input(text, cx);
            window.press(key, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    // Rename in place: the name is preselected, typing replaces it, Enter commits
    cx.update_window(handle, |_, window, cx| window.click(row_g.clone(), cx))
        .unwrap();
    command(cx, Box::new(RenameGroup));
    type_then(cx, "Renamed", "enter");
    // A new group goes under the selected group and starts in rename mode
    command(cx, Box::new(NewGroup));
    type_then(cx, "Prod", "enter");
    assert_eq!(
        group_names(cx),
        [
            ("Renamed".into(), None),
            ("Prod".into(), Some(group.clone()))
        ]
    );

    // Escape cancels the edit
    command(cx, Box::new(RenameGroup));
    type_then(cx, "zzz", "escape");
    assert_eq!(group_names(cx)[1].0, "Prod");

    // A single click selects. Only a double click or the disclosure arrow expands.
    let prod = cx.update(|cx| {
        ConnectionStore::global(cx)
            .read(cx)
            .groups()
            .iter()
            .find(|g| g.name == "Prod")
            .unwrap()
            .id
            .clone()
    });
    let row_prod = format!("g:{prod}");
    cx.update_window(handle, |_, window, cx| {
        if window.try_find(row_prod.clone()).is_some() {
            window.double_click(row_g.clone(), cx);
            window.render_frame(cx);
        }
        window.click(row_g.clone(), cx);
        window.render_frame(cx);
        assert!(
            window.try_find(row_prod.clone()).is_none(),
            "a single click does not expand the group"
        );
        window.click_at(
            row_g.clone(),
            // raw-px: the disclosure chevron is the first 16pt of the row
            point(gpui_kit::px(16.), gpui_kit::px(11.)),
            cx,
        );
        window.render_frame(cx);
        assert!(
            window.try_find(row_prod.clone()).is_some(),
            "the disclosure arrow expands the group"
        );
    })
    .unwrap();

    // Quick filter: only starred connections
    cx.update(|cx| {
        ConnectionStore::global(cx).update(cx, |s, cx| s.update(&a, |c| c.starred = true, cx))
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-filter-starred", cx);
        window.render_frame(cx);
        assert!(window.try_find(row_a.clone()).is_some(), "starred A stays");
        assert!(
            window.try_find(row_b.clone()).is_none(),
            "unstarred B is filtered out"
        );
        window.click("nav-filter-starred", cx);
        window.render_frame(cx);
        assert!(window.try_find(row_b.clone()).is_some());
    })
    .unwrap();
    let _ = host;
    let _ = std::fs::remove_dir_all(home);
}
