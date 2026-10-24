//! Headless UI tests for the result grid: click to select, ⇧ + arrow keys to extend the selection,
//! ⌘C to copy, Return to edit in place, and committing when the current row changes.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::v_flex;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, ElementId, Entity, FocusHandle, InputEvent as _,
    InteractiveElement as _, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, Render, Styled as _, TestAppContext, Window, WindowBounds,
    WindowOptions, div, size,
};
use navidog_db::dialect::Written;
use navidog_grid::model::GridColumn;
use navidog_grid::{GridEvent, ResultGrid};
use navidog_ui::tokens;

fn fit(rem: gpui_kit::Pixels, header: &str, values: &[&str]) -> gpui_kit::Pixels {
    navidog_grid::grid::auto_width(
        header,
        values.iter().copied(),
        tokens::font::SIZE_BODY.to_pixels(rem).as_f32(),
    )
}

fn col(name: &str, pk: bool) -> GridColumn {
    GridColumn {
        name: name.to_string().into(),
        type_name: "".into(),
        primary_key: pk,
        comment: None,
        read_only: false,
    }
}

#[gpui_kit::test]
fn select_copy_and_edit(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_ui::theme::init(cx);
        navidog_grid::init(cx);
    });
    let (handle, grid): (_, Entity<ResultGrid>) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(800.), gpui_kit::px(400.));
        let bounds = gpui_kit::Bounds {
            origin: Default::default(),
            size: test_size,
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ResultGrid::new(window, cx))
        })
        .unwrap()
    });
    cx.update(|cx| {
        grid.update(cx, |g, cx| {
            let cell = |s: &str| Some(s.to_string().into());
            g.set_data(
                vec![col("id", true), col("name", false), col("note", false)],
                vec![
                    vec![cell("1"), cell("a"), None],
                    vec![cell("2"), cell("b"), cell("x")],
                ],
                cx,
            )
        })
    });

    cx.update_window(handle, |_, window, cx| {
        // The horizontal virtual list only knows its visible columns after the first frame's layout
        window.render_frame(cx);
        window.render_frame(cx);
        // Cells are not in the test registry; click the first data column by its offset within
        // the row (column 0 is the marker column)
        let rem = window.rem_size();
        let x = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem) + fit(rem, "id", &["1", "2"]) / 2.;
        window.click_at(
            gpui_kit::ElementId::NamedInteger("row".into(), 0),
            gpui_kit::point(x, rem / 2.),
            cx,
        );
        window.press("shift-down", cx);
        window.press("shift-right", cx);
        window.press("cmd-c", cx);
    })
    .unwrap();
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some("1\ta\n2\tb"));

    // Return edits the selection's active cell (1, 1); select all, type a new value, Return commits
    cx.update_window(handle, |_, window, cx| {
        window.press("enter", cx);
        window.render_frame(cx);
        // The cell editor's select-all is secondary-a (cmd on macOS, ctrl elsewhere).
        window.press("secondary-a", cx);
        window.input("bb", cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let cs = cx.update(|cx| grid.read(cx).model(cx).change_set());
    assert_eq!(cs.updates.len(), 1);
    assert_eq!(
        cs.updates[0].values,
        [("name".to_string(), Written::text(Some("bb".to_string())))]
    );
    assert_eq!(
        cs.updates[0].key.0,
        [("id".to_string(), Some("2".to_string()))]
    );

    // Esc cancels the edit and leaves the value unchanged
    cx.update_window(handle, |_, window, cx| {
        window.press("enter", cx);
        window.render_frame(cx);
        window.input("zzz", cx);
        window.press("escape", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let v = cx.update(|cx| grid.read(cx).model(cx).value(1, 1).cloned());
    assert_eq!(v, Some(Some("bb".into())));

    // A click inside the editor moves the caret and stays in edit mode.
    cx.update_window(handle, |_, window, cx| {
        window.press("enter", cx);
        window.render_frame(cx);
        // The cell editor's select-all is secondary-a (cmd on macOS, ctrl elsewhere).
        window.press("secondary-a", cx);
        window.input("Q", cx);
        let rem = window.rem_size();
        // The edited cell is the second data column (name), past the marker and id.
        let x = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem)
            + fit(rem, "id", &["1", "2"])
            + fit(rem, "name", &["a", "b"]) / 2.;
        window.click_at(
            gpui_kit::ElementId::NamedInteger("row".into(), 1),
            gpui_kit::point(x, rem / 2.),
            cx,
        );
        window.render_frame(cx);
        window.input("Z", cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let v = cx.update(|cx| grid.read(cx).model(cx).value(1, 1).cloned());
    assert_eq!(v, Some(Some("QZ".into())));

    // A drag that starts in the editor must not extend the selection onto another cell.
    cx.update_window(handle, |_, window, cx| {
        window.press("enter", cx);
        window.render_frame(cx);
        let rem = window.rem_size();
        // The editor is the name column: past the marker and the id column.
        let x = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem)
            + fit(rem, "id", &["1", "2"])
            + fit(rem, "name", &["a", "b"]) / 2.;
        let row1 = window
            .find(ElementId::NamedInteger("row".into(), 1))
            .bounds();
        let row0 = window
            .find(ElementId::NamedInteger("row".into(), 0))
            .bounds();
        let origin = gpui_kit::point(row1.origin.x + x, row1.center().y);
        let elsewhere = gpui_kit::point(row0.origin.x + x, row0.center().y);
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: origin,
            modifiers: Modifiers::none(),
            click_count: 1,
            first_mouse: false,
        };
        window.dispatch_event(down.to_platform_input(), cx);
        let move_event = MouseMoveEvent {
            position: elsewhere,
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::none(),
        };
        window.dispatch_event(move_event.to_platform_input(), cx);
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: elsewhere,
            modifiers: Modifiers::none(),
            click_count: 1,
        };
        window.dispatch_event(up.to_platform_input(), cx);
        window.render_frame(cx);
        window.input("!", cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let (head, value) = cx.update(|cx| {
        let m = grid.read(cx).model(cx);
        (m.head(), m.value(1, 1).cloned())
    });
    assert_eq!(head, (1, 1));
    assert_eq!(value, Some(Some("QZ!".into())));

    // A button-down move that did not press a cell (a column drag) leaves the selection.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let rem = window.rem_size();
        let x = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem) + fit(rem, "id", &["1", "2"]) / 2.;
        let row0 = window
            .find(ElementId::NamedInteger("row".into(), 0))
            .bounds();
        let move_event = MouseMoveEvent {
            position: gpui_kit::point(row0.origin.x + x, row0.center().y),
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::none(),
        };
        window.dispatch_event(move_event.to_platform_input(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let head = cx.update(|cx| grid.read(cx).model(cx).head());
    assert_eq!(head, (1, 1));
}

/// The table viewer's column: a pane of fixed size and a grid slot that must not grow
/// with the columns. Mirrors `TableViewer`'s grid slot.
struct ViewerSlot(Entity<ResultGrid>);

impl Render for ViewerSlot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().overflow_hidden().child(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(self.0.clone()),
        )
    }
}

#[gpui_kit::test]
fn wide_tables_scroll_horizontally(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_ui::theme::init(cx);
        navidog_grid::init(cx);
    });
    let (handle, slot) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(600.), gpui_kit::px(300.));
        let bounds = gpui_kit::Bounds {
            origin: Default::default(),
            size: test_size,
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let grid = cx.new(|cx| ResultGrid::new(window, cx));
            cx.new(|_| ViewerSlot(grid))
        })
        .unwrap()
    });
    let grid = cx.update(|cx| slot.read(cx).0.clone());
    let columns = (0..30).map(|i| col(&format!("c{i}"), i == 0)).collect();
    let rows = (0..5)
        .map(|r| (0..30).map(|c| Some(format!("{r}-{c}").into())).collect())
        .collect();
    cx.update(|cx| grid.update(cx, |g, cx| g.set_data(columns, rows, cx)));
    let last_header = gpui_kit::ElementId::NamedInteger("col-header".into(), 30);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(
            window.try_find(last_header.clone()).is_none(),
            "the last column starts off screen"
        );
        // raw-px: a scroll far past the last column
        let dx = gpui_kit::px(-5000.);
        let delta = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(dx, gpui_kit::Pixels::ZERO));
        window.scroll(
            gpui_kit::ElementId::NamedInteger("row".into(), 0),
            delta,
            cx,
        );
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(
            window.try_find(last_header).is_some(),
            "scrolling right reveals the last column"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn leaving_a_modified_row_requests_a_commit(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_ui::theme::init(cx);
        navidog_grid::init(cx);
    });
    let (handle, grid): (_, Entity<ResultGrid>) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(800.), gpui_kit::px(400.));
        let bounds = gpui_kit::Bounds {
            origin: Default::default(),
            size: test_size,
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ResultGrid::new(window, cx))
        })
        .unwrap()
    });
    cx.update(|cx| {
        grid.update(cx, |g, cx| {
            let cell = |s: &str| Some(s.to_string().into());
            g.set_data(
                vec![col("id", true), col("name", false)],
                vec![vec![cell("1"), cell("a")], vec![cell("2"), cell("b")]],
                cx,
            );
            g.update_model(cx, |m| m.set_value(0, 1, Some("z".into())));
        })
    });
    let commits = Rc::new(Cell::new(0));
    let seen = commits.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&grid, move |_, event: &GridEvent, _| {
            if matches!(event, GridEvent::CommitRequested) {
                seen.set(seen.get() + 1);
            }
        })
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let rem = window.rem_size();
        let x = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem) + fit(rem, "id", &["1", "2"]) / 2.;
        window.click_at(
            gpui_kit::ElementId::NamedInteger("row".into(), 1),
            gpui_kit::point(x, rem / 2.),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(commits.get(), 1);
}

/// A right-click focuses the grid. Menu items dispatch to the focused element,
/// so a click while another pane is focused would otherwise do nothing.
struct MenuHost {
    focus: FocusHandle,
    grid: Entity<ResultGrid>,
}

impl Render for MenuHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("grid-menu-host")
            .track_focus(&self.focus)
            .size_full()
            .child(self.grid.clone())
    }
}

#[gpui_kit::test]
fn right_click_focuses_the_grid_so_the_menu_item_runs(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_ui::theme::init(cx);
        navidog_grid::init(cx);
    });
    let (handle, grid, focus): (_, Entity<ResultGrid>, FocusHandle) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(800.), gpui_kit::px(400.));
        let bounds = gpui_kit::Bounds {
            origin: Default::default(),
            size: test_size,
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let (handle, host) = gpui_kit::open_window(options, cx, |window, cx| {
            let grid = cx.new(|cx| ResultGrid::new(window, cx));
            let focus = cx.focus_handle();
            cx.new(|_| MenuHost { focus, grid })
        })
        .unwrap();
        let grid = host.read(cx).grid.clone();
        let focus = host.read(cx).focus.clone();
        (handle, grid, focus)
    });
    cx.update(|cx| {
        grid.update(cx, |g, cx| {
            let cell = |s: &str| Some(s.to_string().into());
            g.set_data(
                vec![col("id", true), col("name", false)],
                vec![vec![cell("1"), cell("a")]],
                cx,
            )
        })
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        window.focus(&focus, cx);
        let rem = window.rem_size();
        // The id column. The name column's estimated offset misses the cell on Linux,
        // and a miss leaves the host focused so Copy never runs.
        let x = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem) + fit(rem, "id", &["1"]) / 2.;
        let row = window
            .find(ElementId::NamedInteger("row".into(), 0))
            .bounds();
        let position = gpui_kit::point(row.origin.x + x, row.origin.y + rem / 2.);
        // Hit testing uses the pointer. A bare button event misses the cell on Linux.
        window.dispatch_event(
            MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Modifiers::none(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Right,
                position,
                modifiers: Modifiers::none(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseUpEvent {
                button: MouseButton::Right,
                position,
                modifiers: Modifiers::none(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    // The table's context menu focuses itself from a defer. Park so that runs
    // before the item, the same window the header menu failed in.
    cx.run_until_parked();
    // RightClickedCell is emitted, so selection and focus land after that update.
    // A native menu leaves that focus in place, so Copy dispatched now hits the grid.
    // Linux draws the menu in the window and focuses it; the Copy item dispatches
    // to the grid captured before the menu opened. Copy follows set-empty, set-null,
    // and a separator.
    cx.update_window(handle, |_, window, cx| {
        #[cfg(target_os = "linux")]
        window.within("popup-menu").click(ElementId::Integer(3), cx);
        #[cfg(not(target_os = "linux"))]
        window.dispatch_action(Box::new(navidog_grid::grid::Copy), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some("1"));
}

/// A column-header right-click focuses the grid. The table's own context menu
/// is empty for a header and used to take focus after this click, so a menu
/// item dispatched once that defer ran never reached the grid.
#[gpui_kit::test]
fn header_right_click_keeps_focus_on_the_grid(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_ui::theme::init(cx);
        navidog_grid::init(cx);
    });
    let (handle, grid, focus): (_, Entity<ResultGrid>, FocusHandle) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(800.), gpui_kit::px(400.));
        let bounds = gpui_kit::Bounds {
            origin: Default::default(),
            size: test_size,
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let (handle, host) = gpui_kit::open_window(options, cx, |window, cx| {
            let grid = cx.new(|cx| ResultGrid::new(window, cx));
            let focus = cx.focus_handle();
            cx.new(|_| MenuHost { focus, grid })
        })
        .unwrap();
        let grid = host.read(cx).grid.clone();
        let focus = host.read(cx).focus.clone();
        (handle, grid, focus)
    });
    cx.update(|cx| {
        grid.update(cx, |g, cx| {
            let cell = |s: &str| Some(s.to_string().into());
            g.set_data(
                vec![col("id", true), col("name", false)],
                vec![vec![cell("1"), cell("a")]],
                cx,
            )
        })
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        window.focus(&focus, cx);
        let header = window
            .find(ElementId::NamedInteger("col-header".into(), 1))
            .bounds();
        // Left side of the header, on the name. The center can land on the sort icon.
        let position = gpui_kit::point(header.origin.x + header.size.width / 4., header.center().y);
        window.dispatch_event(
            MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Modifiers::none(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Right,
                position,
                modifiers: Modifiers::none(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseUpEvent {
                button: MouseButton::Right,
                position,
                modifiers: Modifiers::none(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    // Park so the menu built by the click above is rendered: the Linux item below
    // resolves it through the element registry.
    cx.run_until_parked();
    // A native menu leaves focus on the grid, so the item dispatched now hits it.
    // Linux draws the menu in the window and focuses it; the item dispatches to the
    // grid captured before the menu opened. Copy Field Name follows select-column.
    cx.update_window(handle, |_, window, cx| {
        #[cfg(target_os = "linux")]
        window.within("popup-menu").click(ElementId::Integer(1), cx);
        #[cfg(not(target_os = "linux"))]
        window.dispatch_action(
            Box::new(navidog_grid::grid::CopyFieldName { name: "id".into() }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some("id"));
}

/// The empty tail past the last field is not a cell. Clicking it selects the whole row.
/// A data cell still selects one cell, and the blank row under the records selects nothing.
#[gpui_kit::test]
fn click_past_the_last_column_selects_the_row(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_ui::theme::init(cx);
        navidog_grid::init(cx);
    });
    let (handle, grid): (_, Entity<ResultGrid>) = cx.update(|cx| {
        // raw-px: test window size; wide enough that the columns leave an empty tail
        let test_size = size(gpui_kit::px(800.), gpui_kit::px(400.));
        let bounds = gpui_kit::Bounds {
            origin: Default::default(),
            size: test_size,
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ResultGrid::new(window, cx))
        })
        .unwrap()
    });
    cx.update(|cx| {
        grid.update(cx, |g, cx| {
            let cell = |s: &str| Some(s.to_string().into());
            g.set_data(
                vec![col("id", true), col("name", false)],
                vec![vec![cell("1"), cell("a")], vec![cell("2"), cell("b")]],
                cx,
            )
        })
    });

    let selection = |cx: &mut TestAppContext| cx.update(|cx| grid.read(cx).model(cx).selection());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let rem = window.rem_size();
        // Past the marker, id, and name: the empty tail of the row.
        let tail = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem)
            + fit(rem, "id", &["1", "2"])
            + fit(rem, "name", &["a", "b"])
            // raw-px: click the empty tail past the last column
            + gpui_kit::px(40.);
        let mid = rem / 2.;
        window.click_at(
            ElementId::NamedInteger("row".into(), 0),
            gpui_kit::point(tail, mid),
            cx,
        );
    })
    .unwrap();
    assert_eq!(selection(cx), (0..=0, 0..=1));

    cx.update_window(handle, |_, window, cx| {
        let rem = window.rem_size();
        let id = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem) + fit(rem, "id", &["1", "2"]) / 2.;
        window.click_at(
            ElementId::NamedInteger("row".into(), 1),
            gpui_kit::point(id, rem / 2.),
            cx,
        );
    })
    .unwrap();
    assert_eq!(selection(cx), (1..=1, 0..=0));

    cx.update_window(handle, |_, window, cx| {
        let rem = window.rem_size();
        let tail = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem)
            + fit(rem, "id", &["1", "2"])
            + fit(rem, "name", &["a", "b"])
            // raw-px: click the empty tail past the last column
            + gpui_kit::px(40.);
        // Row 2 is the blank row under the records.
        window.click_at(
            ElementId::NamedInteger("row".into(), 2),
            gpui_kit::point(tail, rem / 2.),
            cx,
        );
    })
    .unwrap();
    assert_eq!(selection(cx), (1..=1, 0..=0));
}

/// A refused visibility change preserves both pending model writes and the active editor.
#[gpui_kit::test]
fn refused_column_visibility_keeps_pending_values_and_the_editor(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_ui::theme::init(cx);
        navidog_grid::init(cx);
    });
    let (handle, grid) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let mut grid = ResultGrid::new(window, cx);
                grid.set_data(
                    vec![col("id", true), col("name", false)],
                    vec![vec![Some("1".into()), Some("a".into())]],
                    cx,
                );
                grid
            })
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        grid.update(cx, |g, cx| {
            g.update_model(cx, |m| m.set_value(0, 0, Some("2".into())));
            g.start_edit((0, 1), window, cx);
        });
        window.render_frame(cx);
        window.press("secondary-a", cx);
        window.input("draft", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        grid.update(cx, |g, cx| {
            assert!(!g.set_hidden_columns(&["id".into()].into(), cx));
            assert_eq!(g.model(cx).columns.len(), 2);
            assert_eq!(g.model(cx).value(0, 0), Some(&Some("2".into())));
        });
        window.render_frame(cx);
        window.input(" continued", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.update(|cx| {
        grid.update(cx, |g, cx| {
            assert_eq!(
                g.model(cx).value(0, 1),
                Some(&Some("draft continued".into()))
            );
            g.update_model(cx, |m| {
                m.discard();
                m.insert_row();
            });
            assert!(g.set_hidden_columns(&["id".into()].into(), cx));
            assert_eq!(g.model(cx).columns[0].name, "name");
            assert_eq!(g.model(cx).row_count(), 2);
            assert_eq!(g.model(cx).value(1, 0), None);
            assert!(g.set_hidden_columns(&Default::default(), cx));
            assert_eq!(g.model(cx).columns.len(), 2);
            assert_eq!(g.model(cx).row_count(), 2);
            assert_eq!(g.model(cx).value(1, 0), None);
            assert_eq!(g.model(cx).value(1, 1), None);
            assert!(g.model(cx).change_set().inserts.is_empty());
        })
    });
}

#[gpui_kit::test]
fn hiding_a_column_keeps_a_pending_write(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_ui::theme::init(cx);
        navidog_grid::init(cx);
    });
    let (_handle, grid): (_, Entity<ResultGrid>) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(800.), gpui_kit::px(400.));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                origin: Default::default(),
                size: test_size,
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ResultGrid::new(window, cx))
        })
        .unwrap()
    });
    let cell = |s: &str| Some(s.to_string().into());
    cx.update(|cx| {
        grid.update(cx, |g, cx| {
            g.set_data(
                vec![col("id", true), col("name", false)],
                vec![vec![cell("1"), cell("a")], vec![cell("2"), cell("b")]],
                cx,
            );
            g.update_model(cx, |m| m.set_value(0, 1, Some("z".into())));
            assert!(!g.set_hidden_columns(&["id".into()].into(), cx));
            let model = g.model(cx);
            assert_eq!(
                model
                    .columns
                    .iter()
                    .map(|c| c.name.as_ref())
                    .collect::<Vec<_>>(),
                ["id", "name"]
            );
            assert!(model.has_changes());
            assert_eq!(model.value(0, 1).and_then(|v| v.as_deref()), Some("z"));
            g.update_model(cx, |m| {
                m.discard();
                m.insert_row();
            });
            assert!(g.set_hidden_columns(&["id".into()].into(), cx));
            let model = g.model(cx);
            assert_eq!(
                model
                    .columns
                    .iter()
                    .map(|c| c.name.as_ref())
                    .collect::<Vec<_>>(),
                ["name"]
            );
            assert_eq!(
                model
                    .all_columns()
                    .map(|c| c.name.as_ref())
                    .collect::<Vec<_>>(),
                ["name", "id"]
            );
            let blank = model.row_count() - 1;
            assert!(model.value(blank, 0).is_none());
            assert_eq!(model.value(0, 0).and_then(|v| v.as_deref()), Some("a"));
        })
    });
}
