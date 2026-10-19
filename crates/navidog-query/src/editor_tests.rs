//! Headless query editor tests on a real SQLite file: running scripts, messages and summary,
//! stopping at errors, result naming, parameters and editing a single-table result.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, EntityInputHandler as _, Focusable as _,
    IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, Window, WindowBounds,
    WindowOptions, div, size,
};
use navidog_core::connection::Connection;
use navidog_core::crypto::Key;
use navidog_core::settings::{ResultsPosition, Settings};
use navidog_core::store::ConnectionStore;
use navidog_db::sessions::Sessions;
use navidog_db::sqlite::SqliteDriver;
use navidog_db::{ConnectParams, Secrets, connect, runtime};

use crate::editor::{QueryEditor, QuerySource, Target};
use crate::results::Body;
use crate::run::RunKind;

struct Host {
    editor: Entity<QueryEditor>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.editor.clone())
    }
}

fn wait(cx: &mut TestAppContext, what: &str, done: impl Fn(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(cx) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn sql(file: &std::path::Path, statements: &str) -> Vec<Vec<Option<String>>> {
    runtime::runtime().block_on(async {
        let params = ConnectParams {
            file: Some(file.to_path_buf()),
            create_new: !file.exists(),
            ..Default::default()
        };
        let mut s = connect(Arc::new(SqliteDriver::default()), params)
            .await
            .unwrap();
        let sets = s.relational().unwrap().execute(statements).await.unwrap();
        s.close().await.unwrap();
        sets.into_iter()
            .last()
            .map(|set| set.rows)
            .unwrap_or_default()
    })
}

fn run(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    editor: &Entity<QueryEditor>,
    kind: RunKind,
) {
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| e.run(kind, window, cx))
    })
    .unwrap();
}

fn idle(cx: &mut TestAppContext, editor: &Entity<QueryEditor>) {
    wait(cx, "the run to finish", |cx| {
        cx.update(|cx| !editor.read(cx).running && editor.read(cx).task.is_some())
    });
}

/// One test at a time. `NAVIDOG_HOME` is process-global, and `name_ok` reads it
/// again later, so the guard must live until the test returns.
fn isolate_home(name: &str) -> (MutexGuard<'static, ()>, std::path::PathBuf) {
    static LOCK: Mutex<()> = Mutex::new(());
    // A panicked test poisons the lock. The next test still needs it, or its own
    // failure is replaced by the poison and never shows up.
    let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let home =
        std::env::temp_dir().join(format!("navidog-query-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: `guard` is held until the caller returns, so no other test reads this variable.
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
    (guard, home)
}

fn set_text(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    editor: &Entity<QueryEditor>,
    text: &str,
) {
    let text = text.to_string();
    cx.update_window(handle, |_, window, cx| {
        let state = editor.read(cx).editor.clone();
        state.update(cx, |s, cx| s.set_value(text, window, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn runs_scripts_into_messages_summary_and_editable_results(cx: &mut TestAppContext) {
    let (_home, home) = isolate_home("scripts");
    let file = home.join("q.db");
    sql(
        &file,
        "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO t(name) VALUES ('a'), ('b')",
    );

    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
        ConnectionStore::global(cx).update(cx, |s, cx| {
            s.set_key(Key::random());
            let mut c = Connection::new("sqlite");
            c.name = "q".into();
            c.general.file = Some(file.to_string_lossy().into_owned());
            let id = c.id.clone();
            s.upsert(c, &[], cx).unwrap();
            id
        })
    });
    cx.executor().allow_parking();
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |s, cx| s.open(id.clone(), Secrets::default(), cx))
    });
    wait(cx, "the connection to open", |cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });

    let script = "-- NAME:people\nSELECT id, name AS n FROM t;\n\
                  UPDATE t SET name = 'B' WHERE id = 2;\n\
                  SELECT * FROM nope;\n\
                  SELECT 1 + 1";
    let (handle, editor) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(1000.), gpui_kit::px(700.));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                origin: Default::default(),
                size: test_size,
            })),
            ..Default::default()
        };
        let mut editor = None;
        let handle = gpui_kit::open_window(options, cx, |window, cx| {
            let e = QueryEditor::new(
                QuerySource::Untitled(1),
                Target {
                    connection: Some(id.clone()),
                    database: Some("main".into()),
                    schema: None,
                },
                script.to_string(),
                window,
                cx,
            );
            editor = Some(e.clone());
            cx.new(|_| Host { editor: e })
        })
        .unwrap();
        (handle.0, editor.unwrap())
    });

    // Stops at the failing statement and marks it
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    cx.update(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.results.rows.len(), 3, "stops after the error");
        assert!(e.results.rows[2].error);
        let names: Vec<_> = e.results.tabs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["people"]);
        let marked = e
            .editor
            .read(cx)
            .diagnostics()
            .map(|d| d.len())
            .unwrap_or(0);
        assert_eq!(marked, 1);
    });
    assert_eq!(
        sql(&file, "SELECT name FROM t WHERE id = 2"),
        [vec![Some("B".to_string())]]
    );

    // The single-table result becomes editable once the primary key is found
    let view = cx.update(|cx| match &editor.read(cx).results.tabs[0].body {
        Body::Grid(v) => v.clone(),
        Body::Explain(_) => panic!("a grid tab"),
    });
    wait(cx, "the result to become editable", |cx| {
        cx.update(|cx| view.read(cx).editable())
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.grid.update(cx, |g, cx| {
                g.update_model(cx, |m| m.set_value(0, 1, Some("z".into())))
            });
            v.commit_now(window, cx);
        })
    })
    .unwrap();
    wait(cx, "the aliased column to be written", |_| {
        sql(&file, "SELECT name FROM t WHERE id = 1") == [vec![Some("z".to_string())]]
    });

    // With "continue on error" the rest runs too; the run drops unpinned tabs first
    cx.update(|cx| Settings::update(cx, |s| s.continue_on_error = true));
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    cx.update(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.results.rows.len(), 4);
        assert_eq!(e.results.tabs.len(), 2);
        assert_eq!(
            e.results.tabs[1].name,
            rust_i18n::t!("query.result_n", n = 2)
        );
    });

    // Selected/current execution must not run the other statements in the document.
    let script = "SELECT 11; SELECT 22; SELECT 33;";
    set_text(cx, handle, &editor, script);
    cx.update(|cx| {
        editor.read(cx).editor.clone().update(cx, |s, cx| {
            s.set_selected_range(11..20, cx);
        });
    });
    run(cx, handle, &editor, RunKind::Selected);
    idle(cx, &editor);
    cx.update(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.results.rows.len(), 1);
        assert_eq!(e.results.tabs.len(), 1);
        let Body::Grid(view) = &e.results.tabs[0].body else {
            panic!("grid result")
        };
        assert_eq!(view.read(cx).rows, [vec![Some("22".into())]]);
    });
    cx.update(|cx| {
        editor.read(cx).editor.clone().update(cx, |s, cx| {
            s.set_selected_range(25..25, cx);
        });
    });
    run(cx, handle, &editor, RunKind::Current);
    idle(cx, &editor);
    cx.update(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.results.rows.len(), 1);
        let Body::Grid(view) = &e.results.tabs[0].body else {
            panic!("grid result")
        };
        assert_eq!(view.read(cx).rows, [vec![Some("33".into())]]);
    });

    // The result limit must not truncate execution or its summary.
    set_text(cx, handle, &editor, &"SELECT 1;".repeat(21));
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    cx.update(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.results.tabs.len(), 20);
        assert_eq!(e.results.rows.len(), 21);
    });

    // Cancel only after the SQLite session is executing, then skip the next statement.
    set_text(
        cx,
        handle,
        &editor,
        "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n; SELECT 99;",
    );
    run(cx, handle, &editor, RunKind::All);
    wait(cx, "SQLite to hold the running session", |cx| {
        cx.update(|cx| {
            editor
                .read(cx)
                .live
                .as_ref()
                .is_some_and(|live| live.session.try_lock().is_err())
        })
    });
    cx.update_window(handle, |_, window, cx| {
        editor
            .read(cx)
            .editor
            .clone()
            .update(cx, |s, cx| s.focus(window, cx));
        window.render_frame(cx);
        window.press("cmd-.", cx);
    })
    .unwrap();
    idle(cx, &editor);
    cx.update(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.results.rows.len(), 1);
        assert!(e.results.rows[0].error);
    });

    // Parameters: the dialog is prefilled with the last value; raw mode skips the quotes
    set_text(cx, handle, &editor, "SELECT [$v] AS v, [$w] AS w");
    cx.update(|cx| {
        editor.update(cx, |e, _| {
            e.params.insert("v".into(), ("it's".into(), false));
            e.params.insert("w".into(), ("1 + 2".into(), true));
        })
    });
    run(cx, handle, &editor, RunKind::All);
    // Enter confirms the parameter dialog; a click on the footer can miss it.
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    idle(cx, &editor);
    let view = cx.update(
        |cx| match &editor.read(cx).results.tabs.last().unwrap().body {
            Body::Grid(v) => v.clone(),
            Body::Explain(_) => panic!("a grid tab"),
        },
    );
    cx.update(|cx| {
        let rows = view.read(cx).rows.clone();
        assert_eq!(
            rows,
            [vec![Some("it's".to_string()), Some("3".to_string())]]
        );
    });

    // Explain on SQLite opens an explain tab
    set_text(cx, handle, &editor, "SELECT * FROM t WHERE id = 1");
    run(cx, handle, &editor, RunKind::Explain);
    idle(cx, &editor);
    cx.update(|cx| {
        let e = editor.read(cx);
        assert!(matches!(
            e.results.tabs.last().unwrap().body,
            Body::Explain(_)
        ));
    });

    // An existing result stays on the new page; back returns to the editor; show results returns.
    let go = |cx: &mut TestAppContext, action: Box<dyn gpui_kit::Action>| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            window.dispatch_action(action, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    go(cx, Box::new(crate::editor::ResultsNewPage));
    cx.update(|cx| assert!(editor.read(cx).results_page));
    go(cx, Box::new(crate::results::BackToEditor));
    cx.update(|cx| assert!(!editor.read(cx).results_page));
    go(cx, Box::new(crate::editor::ShowResults));
    cx.update(|cx| assert!(editor.read(cx).results_page));

    // Focus stays on the SQL editor the results page just unmounted. The position
    // buttons must still switch layout; dispatching from that focus would miss.
    cx.update_window(handle, |_, _, cx| {
        editor.update(cx, |e, cx| {
            e.results_page = false;
            cx.notify();
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        editor.update(cx, |e, cx| {
            e.editor.update(cx, |ed, cx| ed.focus(window, cx));
            e.results_page = true;
            cx.notify();
        });
        Settings::update(cx, |s| s.results_position = ResultsPosition::NewPage);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("result-below", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            Settings::global(cx).results_position,
            ResultsPosition::Below
        );
        assert!(!editor.read(cx).results_page);
    });
    cx.update_window(handle, |_, _, cx| {
        editor.update(cx, |e, cx| {
            e.results_page = true;
            cx.notify();
        });
        Settings::update(cx, |s| s.results_position = ResultsPosition::NewPage);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("result-back", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(!editor.read(cx).results_page));

    // Without keep_results the next run replaces unpinned tabs, and they have no close button.
    cx.update(|cx| Settings::update(cx, |s| s.results_position = ResultsPosition::Below));
    set_text(cx, handle, &editor, "SELECT 1 AS n");
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    let replaced = cx.update(|cx| editor.read(cx).results.tabs[0].id);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find(gpui_kit::SharedString::from(format!(
                    "result-close-{replaced}"
                )))
                .is_none()
        );
    })
    .unwrap();
    set_text(cx, handle, &editor, "SELECT 2 AS n");
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    let kept = cx.update(|cx| {
        let tabs = &editor.read(cx).results.tabs;
        assert_eq!(tabs.len(), 1);
        assert_ne!(tabs[0].id, replaced);
        tabs[0].id
    });

    // With it on, the previous tab stays, the new one is current, and its close button works.
    cx.update(|cx| Settings::update(cx, |s| s.keep_results = true));
    set_text(cx, handle, &editor, "SELECT 3 AS n");
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    let current = cx.update(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.results.tabs.len(), 2);
        assert_eq!(e.results.tabs[0].id, kept);
        assert_eq!(e.results.rows.len(), 1);
        let current = e.results.tabs[1].id;
        assert!(matches!(
            e.results.page,
            crate::results::Page::Tab(id) if id == current
        ));
        current
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let id = gpui_kit::SharedString::from(format!("result-close-{kept}"));
        assert!(window.try_find(id.clone()).is_some());
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.results.tabs.len(), 1);
        assert_eq!(e.results.tabs[0].id, current);
        assert!(matches!(
            e.results.page,
            crate::results::Page::Tab(id) if id == current
        ));
    });

    // The context menu dispatches along the focused SQL editor and bubbles to this root.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        editor.update(cx, |e, cx| {
            e.editor.update(cx, |ed, cx| ed.focus(window, cx));
        });
        window.dispatch_action(Box::new(crate::results::TogglePinTab { id: current }), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(editor.read(cx).results.tabs[0].pinned));
    // A pin keeps that result when the next run would otherwise replace it.
    cx.update(|cx| Settings::update(cx, |s| s.keep_results = false));
    set_text(cx, handle, &editor, "SELECT 4 AS n");
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    cx.update(|cx| {
        let tabs = &editor.read(cx).results.tabs;
        assert!(tabs.iter().any(|t| t.id == current && t.pinned));
    });
}

#[gpui_kit::test]
fn typing_a_character_does_not_reenter_the_editor(cx: &mut TestAppContext) {
    let (_home, _dir) = isolate_home("typing");
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
    });
    let (handle, editor) = cx.update(|cx| {
        let mut editor = None;
        let handle = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let e = QueryEditor::new(
                QuerySource::Untitled(1),
                Target::default(),
                String::new(),
                window,
                cx,
            );
            editor = Some(e.clone());
            cx.new(|_| Host { editor: e })
        })
        .unwrap();
        (handle.0, editor.unwrap())
    });
    // The same path as macOS insertText: inside the editor update, while completion is on.
    cx.update_window(handle, |_, window, cx| {
        let state = editor.read(cx).editor.clone();
        state.update(cx, |s, cx| s.replace_text_in_range(None, "s", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| editor.read(cx).text(cx)), "s");
}

#[gpui_kit::test]
fn format_commands_undo_in_one_step_and_the_stack_cycles(cx: &mut TestAppContext) {
    let (_home, _dir) = isolate_home("format");
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
    });
    let original = "select a\nselect b";
    let (handle, editor) = cx.update(|cx| {
        let mut editor = None;
        let handle = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let e = QueryEditor::new(
                QuerySource::Untitled(1),
                Target::default(),
                original.to_string(),
                window,
                cx,
            );
            editor = Some(e.clone());
            cx.new(|_| Host { editor: e })
        })
        .unwrap();
        (handle.0, editor.unwrap())
    });
    let text = |cx: &mut TestAppContext| cx.update(|cx| editor.read(cx).text(cx));
    let state = cx.update(|cx| editor.read(cx).editor.clone());
    let undo = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            state.update(cx, |s, cx| s.focus(window, cx));
            window.render_frame(cx);
            // Undo is cmd-z on macOS and ctrl-z elsewhere.
            window.press("secondary-z", cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let select = |cx: &mut TestAppContext, range: std::ops::Range<usize>| {
        cx.update(|cx| state.update(cx, |s, cx| s.set_selected_range(range, cx)));
    };

    select(cx, 3..12);
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| {
            e.comment(navidog_util::sql_format::CommentMode::Toggle, window, cx)
        })
    })
    .unwrap();
    assert_eq!(text(cx), "-- select a\n-- select b");
    undo(cx);
    assert_eq!(text(cx), original);

    select(cx, 0..8);
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| e.change_case(true, window, cx))
    })
    .unwrap();
    assert_eq!(text(cx), "SELECT A\nselect b");
    undo(cx);
    assert_eq!(text(cx), original);

    select(cx, 0..0);
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| e.beautify(window, cx))
    })
    .unwrap();
    assert!(text(cx).starts_with("SELECT"), "{}", text(cx));
    undo(cx);
    assert_eq!(text(cx), original);

    // Regular expression find over the whole text
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| {
            e.open_find(false, window, cx);
            e.find.options.regex = true;
            e.find
                .query
                .update(cx, |s, cx| s.set_value(r"sel\w+ b", window, cx));
            e.refresh_matches(cx);
        })
    })
    .unwrap();
    cx.update(|cx| assert_eq!(editor.read(cx).find.matches, vec![9..17]));

    // The stack pastes its newest item, then swaps in the older ones
    cx.update(|cx| {
        let stack = cx.default_global::<crate::format::ClipboardStack>();
        stack.push("one".into());
        stack.push("two".into());
    });
    let end = original.len();
    select(cx, end..end);
    let paste = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            editor.update(cx, |e, cx| e.paste_from_stack(window, cx))
        })
        .unwrap();
    };
    paste(cx);
    assert_eq!(text(cx), format!("{original}two"));
    paste(cx);
    assert_eq!(text(cx), format!("{original}one"));
    paste(cx);
    assert_eq!(text(cx), format!("{original}two"));

    // A snippet's placeholders become tab stops: the first is selected, Tab moves on, then ends
    select(cx, 0..0);
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| {
            e.insert_text("SELECT ${cols} FROM ${t};\n", true, window, cx)
        })
    })
    .unwrap();
    let selected =
        |cx: &mut TestAppContext| cx.update(|cx| state.read(cx).selected_text().to_string());
    assert_eq!(selected(cx), "cols");
    let tab = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            editor.update(cx, |e, cx| e.step_tab_stop(true, window, cx))
        })
        .unwrap()
    };
    assert!(tab(cx));
    assert_eq!(selected(cx), "t");
    assert!(tab(cx), "past the last stop the jumps end");
    assert!(cx.update(|cx| editor.read(cx).tab_stops.is_none()));
}

#[gpui_kit::test]
fn saves_untitled_queries_reloads_outside_edits_and_recovers(cx: &mut TestAppContext) {
    let (_home, _dir) = isolate_home("save");
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
        // Reduced motion settles the dialog entrance on its first frame. A click
        // samples one frame's bounds and hit tests them against the next, so an
        // entrance still sliding on the real clock can miss a small button.
        cx.set_reduce_motion(true);
    });
    let scope = navidog_core::object::Scope::new("save-test", "main", None);
    let _ = navidog_core::queries::delete(&scope, "first");
    navidog_core::queries::write(&scope, "taken", "SELECT 0").unwrap();
    let (handle, editor) = cx.update(|cx| {
        // raw-px: wider than the 870pt recovery dialog, so OK is not on the mask
        let test_size = size(gpui_kit::px(1200.), gpui_kit::px(800.));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                origin: Default::default(),
                size: test_size,
            })),
            ..Default::default()
        };
        let mut editor = None;
        let handle = gpui_kit::open_window(options, cx, |window, cx| {
            let e = QueryEditor::new(
                QuerySource::Untitled(1),
                Target {
                    connection: Some("save-test".into()),
                    database: Some("main".into()),
                    schema: None,
                },
                "SELECT 1".to_string(),
                window,
                cx,
            );
            editor = Some(e.clone());
            cx.new(|_| Host { editor: e })
        })
        .unwrap();
        (handle.0, editor.unwrap())
    });
    cx.update(|cx| editor.update(cx, |e, _| e.dirty = true));

    // A taken name is refused; a free one saves and binds the tab to the query
    let name_and_ok = |cx: &mut TestAppContext, name: &str| {
        let name = name.to_string();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("query-name-input", cx);
            window.press("secondary-a", cx);
            window.input(&name, cx);
            window.click("query-name-ok", cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| e.save_query(false, window, cx))
    })
    .unwrap();
    name_and_ok(cx, "taken");
    assert!(
        cx.update(|cx| editor.read(cx).dirty),
        "a taken name does not save"
    );
    name_and_ok(cx, "first");
    assert_eq!(
        navidog_core::queries::read(&scope, "first").unwrap(),
        "SELECT 1"
    );
    cx.update(|cx| {
        let e = editor.read(cx);
        assert!(!e.dirty);
        assert!(matches!(&e.source, QuerySource::Saved(o) if o.name == "first"));
    });

    // An edit made outside is offered for reload when the tab is activated
    std::thread::sleep(Duration::from_millis(1100));
    navidog_core::queries::write(&scope, "first", "SELECT 2").unwrap();
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| e.check_external_change(window, cx));
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| editor.read(cx).text(cx)), "SELECT 2");

    // A crash leftover comes back through the dialog with its text, unsaved
    crate::recovery::write(&crate::recovery::Autosave {
        id: "crashed".into(),
        title: "Untitled 9".into(),
        source: QuerySource::Untitled(9),
        target: Target::default(),
        text: "SELECT 'lost'".into(),
        saved_at: "2026-10-11 00:00:00".into(),
        on_quit: false,
    });
    let recovered: std::rc::Rc<std::cell::RefCell<Vec<Entity<QueryEditor>>>> = Default::default();
    let sink = recovered.clone();
    cx.update_window(handle, |_, window, cx| {
        crate::recovery::check(window, cx, move |e, _, _| sink.borrow_mut().push(e));
        window.render_frame(cx);
    })
    .unwrap();
    // Reduced motion settles the dialog on its first frame, so the click cannot
    // miss a button that the entrance slide is still moving.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("recover-ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let recovered = recovered.borrow();
    let back = recovered
        .iter()
        .find(|e| cx.update(|cx| e.read(cx).id == "crashed"))
        .expect("recovered");
    cx.update(|cx| {
        assert_eq!(back.read(cx).text(cx), "SELECT 'lost'");
        assert!(back.read(cx).dirty);
    });
}

#[gpui_kit::test]
fn history_log_follows_the_log_and_the_console_runs_statements(cx: &mut TestAppContext) {
    let (_home, home) = isolate_home("console");
    let file = home.join("console.db");
    sql(
        &file,
        "CREATE TABLE c(id INTEGER PRIMARY KEY, v TEXT); INSERT INTO c(v) VALUES ('x')",
    );
    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
        ConnectionStore::global(cx).update(cx, |s, cx| {
            s.set_key(Key::random());
            let mut c = Connection::new("sqlite");
            c.name = "console".into();
            c.general.file = Some(file.to_string_lossy().into_owned());
            let id = c.id.clone();
            s.upsert(c, &[], cx).unwrap();
            id
        })
    });
    cx.executor().allow_parking();
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |s, cx| s.open(id.clone(), Secrets::default(), cx))
    });
    wait(cx, "the connection", |cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });

    struct Two {
        log: Entity<crate::HistoryLog>,
        console: Entity<crate::Console>,
    }
    impl Render for Two {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.log.clone())
                .child(self.console.clone())
        }
    }
    let (handle, log, console) = cx.update(|cx| {
        let mut out = None;
        let h = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let log = crate::HistoryLog::new(window, cx);
            let console = crate::Console::new(id.clone(), Some("main".into()), window, cx);
            out = Some((log.clone(), console.clone()));
            cx.new(|_| Two { log, console })
        })
        .unwrap();
        let (l, c) = out.unwrap();
        (h.0, l, c)
    });
    wait(cx, "the console session", |cx| {
        cx.update(|cx| console.read(cx).has_session())
    });

    // An incomplete statement only gets a new line; with the terminator it runs
    let type_and_enter = |cx: &mut TestAppContext, text: &str| {
        let text = text.to_string();
        cx.update_window(handle, |_, window, cx| {
            let input = console.read(cx).input_state();
            input.update(cx, |s, cx| {
                let len = s.value().len();
                s.set_selected_range(len..len, cx);
                s.insert(text, window, cx);
                s.focus(window, cx);
            });
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    type_and_enter(cx, "SELECT v FROM c");
    assert_eq!(
        cx.update(|cx| console.read(cx).input_state().read(cx).value().to_string()),
        "SELECT v FROM c\n"
    );
    type_and_enter(cx, "WHERE id = 1;");
    wait(cx, "the console output", |cx| {
        cx.update(|cx| console.read(cx).output_text().contains("1 row in set"))
    });
    let out = cx.update(|cx| console.read(cx).output_text());
    assert!(
        out.contains("sqlite> SELECT v FROM c\n   ...> WHERE id = 1;"),
        "{out}"
    );
    assert!(out.contains("| x |"), "{out}");

    // History navigation goes through the editor's real key handlers.
    for (key, expected) in [("up", "SELECT v FROM c\nWHERE id = 1;"), ("down", "")] {
        cx.update_window(handle, |_, window, cx| {
            let input = console.read(cx).input_state();
            input.update(cx, |s, cx| {
                // Up only walks history at the first line; Down at the last.
                let caret = if key == "up" { 0 } else { s.value().len() };
                s.set_selected_range(caret..caret, cx);
                s.focus(window, cx);
            });
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            cx.update(|cx| console.read(cx).input_state().read(cx).value().to_string()),
            expected
        );
    }

    type_and_enter(
        cx,
        "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n;",
    );
    wait(cx, "the console statement to execute", |cx| {
        cx.update(|cx| console.read(cx).session_busy())
    });
    cx.update_window(handle, |_, window, cx| {
        let input = console.read(cx).input_state();
        input.update(cx, |s, cx| s.focus(window, cx));
        window.render_frame(cx);
        window.press("cmd-.", cx);
    })
    .unwrap();
    wait(cx, "the console cancellation", |cx| {
        cx.update(|cx| console.read(cx).output_text().contains("ERROR"))
    });
    type_and_enter(cx, "SELECT 42 AS resumed;");
    wait(cx, "the console to run after cancellation", |cx| {
        cx.update(|cx| console.read(cx).output_text().contains("| 42 "))
    });

    // The history log picks the statement up on its next poll
    wait(cx, "the history log", |cx| {
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
        cx.update(|cx| {
            log.read(cx)
                .text(cx)
                .contains("SELECT v FROM c\nWHERE id = 1")
        })
    });
}

/// Exercises the query editor against the disposable Compose databases.
#[gpui_kit::test]
fn live_editor_runs_selection_current_errors_and_stop(cx: &mut TestAppContext) {
    if std::env::var_os("NAVIDOG_LIVE").is_none() {
        return;
    }
    let (_home, _) = isolate_home("live-editor");
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
        ConnectionStore::global(cx).update(cx, |s, _| s.set_key(Key::random()));
    });
    for (driver, port, database, slow) in [
        ("mysql", 13306, "sakila", "SELECT SLEEP(30)"),
        ("mariadb", 13307, "sakila", "SELECT SLEEP(30)"),
        ("postgres", 15432, "pagila", "SELECT pg_sleep(30)"),
    ] {
        let id = cx.update(|cx| {
            ConnectionStore::global(cx).update(cx, |s, cx| {
                let mut c = Connection::new(driver);
                c.name = format!("acceptance-{driver}");
                c.general.host = "127.0.0.1".into();
                c.general.port = Some(port);
                c.general.database = Some(database.into());
                let key = s.key().unwrap();
                navidog_core::store::ConnectionsFile::set_secret(
                    &mut c,
                    navidog_core::connection::SecretField::Password,
                    Some("navidog"),
                    &key,
                );
                let id = c.id.clone();
                s.upsert(c, &[], cx).unwrap();
                id
            })
        });
        cx.update(|cx| {
            Sessions::global(cx).update(cx, |s, cx| {
                s.open(
                    id.clone(),
                    Secrets {
                        password: Some("navidog".into()),
                        ..Default::default()
                    },
                    cx,
                )
            })
        });
        wait(cx, "the live connection", |cx| {
            cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
        });
        let (handle, editor) = cx.update(|cx| {
            let mut editor = None;
            let handle = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let e = QueryEditor::new(
                    QuerySource::Untitled(1),
                    Target {
                        connection: Some(id.clone()),
                        database: Some(database.into()),
                        schema: None,
                    },
                    String::new(),
                    window,
                    cx,
                );
                editor = Some(e.clone());
                cx.new(|_| Host { editor: e })
            })
            .unwrap();
            (handle.0, editor.unwrap())
        });
        for (kind, range, value) in [
            (RunKind::Selected, 11..20, "22"),
            (RunKind::Current, 25..25, "33"),
        ] {
            set_text(cx, handle, &editor, "SELECT 11; SELECT 22; SELECT 33;");
            cx.update(|cx| {
                editor
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_selected_range(range, cx))
            });
            run(cx, handle, &editor, kind);
            idle(cx, &editor);
            cx.update(|cx| {
                let e = editor.read(cx);
                assert_eq!(e.results.rows.len(), 1, "{driver}");
                let Body::Grid(view) = &e.results.tabs[0].body else {
                    panic!("grid")
                };
                assert_eq!(view.read(cx).rows, [vec![Some(value.into())]], "{driver}");
            });
        }
        if driver != "postgres" {
            set_text(
                cx,
                handle,
                &editor,
                "DROP PROCEDURE IF EXISTS navidog_acceptance_proc;\nDELIMITER $$\nCREATE PROCEDURE navidog_acceptance_proc() BEGIN SELECT 7 AS n; END$$\nDELIMITER ;\n-- NAME:routine\nCALL navidog_acceptance_proc();\nDROP PROCEDURE navidog_acceptance_proc;",
            );
            run(cx, handle, &editor, RunKind::All);
            idle(cx, &editor);
            cx.update(|cx| {
                let e = editor.read(cx);
                assert!(e.results.rows.iter().all(|row| !row.error), "{driver}");
                assert_eq!(e.results.tabs[0].name, "routine");
                let Body::Grid(view) = &e.results.tabs[0].body else {
                    panic!("grid")
                };
                assert_eq!(view.read(cx).rows, [vec![Some("7".into())]]);
            });
        }
        for (continue_on_error, count) in [(false, 2), (true, 3)] {
            cx.update(|cx| Settings::update(cx, |s| s.continue_on_error = continue_on_error));
            set_text(
                cx,
                handle,
                &editor,
                "SELECT 1; SELECT * FROM navidog_missing_acceptance_table; SELECT 2;",
            );
            run(cx, handle, &editor, RunKind::All);
            idle(cx, &editor);
            cx.update(|cx| {
                let e = editor.read(cx);
                assert_eq!(e.results.rows.len(), count, "{driver}");
                assert!(e.results.rows[1].error);
                assert_eq!(e.editor.read(cx).diagnostics().unwrap().len(), 1);
            });
        }
        set_text(cx, handle, &editor, &format!("{slow}; SELECT 99;"));
        run(cx, handle, &editor, RunKind::All);
        wait(cx, "the cancellable live session", |cx| {
            cx.update(|cx| editor.read(cx).live.is_some())
        });
        wait(cx, "the server to execute the slow statement", |_| {
            runtime::runtime().block_on(async {
                let driver: Arc<dyn navidog_db::Driver> = match driver {
                    "postgres" => Arc::new(navidog_db::postgres::PostgresDriver::default()),
                    "mariadb" => Arc::new(navidog_db::mysql::MySqlDriver::mariadb()),
                    _ => Arc::new(navidog_db::mysql::MySqlDriver::mysql()),
                };
                let mut session = connect(driver, ConnectParams {
                    host: "127.0.0.1".into(), port,
                    user: if port == 15432 { "postgres" } else { "root" }.into(),
                    password: Some("navidog".into()), database: Some(database.into()),
                    ..Default::default()
                }).await.unwrap();
                let check = if port == 15432 {
                    "SELECT COUNT(*) FROM pg_stat_activity WHERE state = 'active' AND query LIKE 'SELECT pg_sleep(30)%'"
                } else {
                    "SELECT COUNT(*) FROM information_schema.PROCESSLIST WHERE INFO LIKE 'SELECT SLEEP(30)%'"
                };
                let sets = session.relational().unwrap().execute(check).await.unwrap();
                let active = sets[0].rows[0][0].as_deref() != Some("0");
                session.close().await.unwrap();
                active
            })
        });
        cx.update_window(handle, |_, window, cx| {
            editor
                .read(cx)
                .editor
                .clone()
                .update(cx, |s, cx| s.focus(window, cx));
            window.render_frame(cx);
            window.press("cmd-.", cx);
        })
        .unwrap();
        idle(cx, &editor);
        cx.update(|cx| {
            assert_eq!(
                editor.read(cx).results.rows.len(),
                1,
                "{driver}: stop skips following statements"
            )
        });
    }
}
