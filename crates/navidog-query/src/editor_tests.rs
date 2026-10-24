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
use navidog_core::workspace::{Workspace, WorkspaceEvent};
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

fn live_sql(driver: &str, port: u16, database: &str, statements: &str) {
    runtime::runtime().block_on(async {
        let driver: Arc<dyn navidog_db::Driver> = match driver {
            "postgres" => Arc::new(navidog_db::postgres::PostgresDriver::default()),
            "mariadb" => Arc::new(navidog_db::mysql::MySqlDriver::mariadb()),
            _ => Arc::new(navidog_db::mysql::MySqlDriver::mysql()),
        };
        let mut session = connect(
            driver,
            ConnectParams {
                host: "127.0.0.1".into(),
                port,
                user: if port == 15432 { "postgres" } else { "root" }.into(),
                password: Some("navidog".into()),
                database: Some(database.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        session
            .relational()
            .unwrap()
            .execute(statements)
            .await
            .unwrap();
        session.close().await.unwrap();
    });
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

fn enter_console(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    console: &Entity<crate::Console>,
    text: &str,
) {
    let text = text.to_string();
    cx.update_window(handle, |_, window, cx| {
        let input = console.read(cx).input_state();
        input.update(cx, |state, cx| {
            let len = state.value().len();
            state.set_selected_range(len..len, cx);
            state.insert(text, window, cx);
            state.focus(window, cx);
        });
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn closing_query_and_console_windows_preserves_in_flight_refreshes(cx: &mut TestAppContext) {
    let (_home, home) = isolate_home("closed-run");
    let file = home.join("fixture.db");
    sql(
        &file,
        "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t VALUES (1, 'initial')",
    );
    cx.executor().allow_parking();
    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
        ConnectionStore::global(cx).update(cx, |store, cx| {
            store.set_key(Key::random());
            let mut connection = Connection::new("sqlite");
            connection.general.file = Some(file.to_string_lossy().into_owned());
            let id = connection.id.clone();
            store.upsert(connection, &[], cx).unwrap();
            id
        })
    });
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |sessions, cx| {
            sessions.open(id.clone(), Secrets::default(), cx)
        });
    });
    wait(cx, "the connection", |cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });
    let changes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(
            &Workspace::global(cx),
            move |_, event: &WorkspaceEvent, _| {
                if let WorkspaceEvent::DatabaseChanged(connection) = event {
                    observed.borrow_mut().push(connection.clone());
                }
            },
        )
    });
    let block_writes = || {
        runtime::runtime()
            .block_on(connect(
                Arc::new(SqliteDriver::default()),
                ConnectParams {
                    file: Some(file.clone()),
                    initial_queries: "BEGIN IMMEDIATE".into(),
                    ..Default::default()
                },
            ))
            .unwrap()
    };
    let release_writes = |mut blocker: Box<dyn navidog_db::Session>| {
        runtime::runtime().block_on(async move {
            blocker
                .relational()
                .unwrap()
                .execute("COMMIT")
                .await
                .unwrap();
            blocker.close().await.unwrap();
        });
    };

    let (handle, editor) = cx.update(|cx| {
        let mut editor = None;
        let handle = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let view = QueryEditor::new(
                QuerySource::Untitled(1),
                Target {
                    connection: Some(id.clone()),
                    database: Some("main".into()),
                    schema: None,
                },
                "PRAGMA busy_timeout=30000".into(),
                window,
                cx,
            );
            editor = Some(view.clone());
            cx.new(|_| Host { editor: view })
        })
        .unwrap();
        (handle.0, editor.unwrap())
    });
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    wait(cx, "the query setup notification", |_| {
        changes.borrow().len() == 1
    });
    changes.borrow_mut().clear();
    let blocker = block_writes();
    set_text(cx, handle, &editor, "UPDATE t SET name = 'query closed'");
    run(cx, handle, &editor, RunKind::All);
    wait(cx, "the blocked query", |cx| {
        cx.update(|cx| {
            editor
                .read(cx)
                .live
                .as_ref()
                .is_some_and(|live| live.session.try_lock().is_err())
        })
    });
    let closed = editor.downgrade();
    cx.update_window(handle, |_, window, _| window.remove_window())
        .unwrap();
    drop(editor);
    cx.run_until_parked();
    assert!(
        closed.upgrade().is_none(),
        "the query and its task were released"
    );
    assert!(
        changes.borrow().is_empty(),
        "refresh waits for the database job"
    );
    release_writes(blocker);
    wait(cx, "the closed query's completion notification", |_| {
        changes.borrow().len() == 1
    });
    assert_eq!(changes.borrow().as_slice(), std::slice::from_ref(&id));
    assert_eq!(
        sql(&file, "SELECT name FROM t"),
        [vec![Some("query closed".to_string())]]
    );

    changes.borrow_mut().clear();
    let (handle, console) = cx.update(|cx| {
        let mut console = None;
        let handle = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let view = crate::Console::new(id.clone(), Some("main".into()), window, cx);
            console = Some(view.clone());
            view
        })
        .unwrap();
        (handle.0, console.unwrap())
    });
    wait(cx, "the console session", |cx| {
        cx.update(|cx| console.read(cx).has_session())
    });
    enter_console(cx, handle, &console, "PRAGMA busy_timeout=30000;");
    wait(cx, "the console setup notification", |_| {
        changes.borrow().len() == 1
    });
    cx.run_until_parked();
    changes.borrow_mut().clear();
    let blocker = block_writes();
    enter_console(
        cx,
        handle,
        &console,
        "UPDATE t SET name = 'console closed';",
    );
    wait(cx, "the blocked console input", |cx| {
        cx.update(|cx| console.read(cx).session_busy())
    });
    let closed = console.downgrade();
    cx.update_window(handle, |_, window, _| window.remove_window())
        .unwrap();
    drop(console);
    cx.run_until_parked();
    assert!(
        closed.upgrade().is_none(),
        "the console and its task were released"
    );
    assert!(
        changes.borrow().is_empty(),
        "refresh waits for the database job"
    );
    release_writes(blocker);
    wait(cx, "the closed console's completion notification", |_| {
        changes.borrow().len() == 1
    });
    cx.run_until_parked();
    assert_eq!(changes.borrow().as_slice(), std::slice::from_ref(&id));
    assert_eq!(
        sql(&file, "SELECT name FROM t"),
        [vec![Some("console closed".to_string())]]
    );
}

#[gpui_kit::test]
fn query_runs_publish_one_refresh_for_the_original_connection(cx: &mut TestAppContext) {
    let (_home, home) = isolate_home("database-changes");
    let first_file = home.join("first.db");
    let second_file = home.join("second.db");
    for file in [&first_file, &second_file] {
        sql(
            file,
            "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t VALUES (1, 'initial')",
        );
    }
    cx.executor().allow_parking();
    let ids = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
        Settings::update(cx, |settings| settings.continue_on_error = false);
        ConnectionStore::global(cx).update(cx, |store, cx| {
            store.set_key(Key::random());
            [&first_file, &second_file]
                .into_iter()
                .map(|file| {
                    let mut connection = Connection::new("sqlite");
                    connection.general.file = Some(file.to_string_lossy().into_owned());
                    connection.general.database = Some("main".into());
                    let id = connection.id.clone();
                    store.upsert(connection, &[], cx).unwrap();
                    id
                })
                .collect::<Vec<_>>()
        })
    });
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |sessions, cx| {
            for id in &ids {
                sessions.open(id.clone(), Secrets::default(), cx);
            }
        });
    });
    wait(cx, "both connections", |cx| {
        cx.update(|cx| {
            ids.iter()
                .all(|id| Sessions::global(cx).read(cx).state(id).is_open())
        })
    });
    let (handle, editor) = cx.update(|cx| {
        let mut editor = None;
        let handle = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let view = QueryEditor::new(
                QuerySource::Untitled(1),
                Target {
                    connection: Some(ids[0].clone()),
                    database: Some("main".into()),
                    schema: None,
                },
                String::new(),
                window,
                cx,
            );
            editor = Some(view.clone());
            cx.new(|_| Host { editor: view })
        })
        .unwrap();
        (handle.0, editor.unwrap())
    });
    let changes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(
            &Workspace::global(cx),
            move |_, event: &WorkspaceEvent, _| {
                if let WorkspaceEvent::DatabaseChanged(connection) = event {
                    observed.borrow_mut().push(connection.clone());
                }
            },
        )
    });
    let run_once = |cx: &mut TestAppContext, text: &str| {
        changes.borrow_mut().clear();
        set_text(cx, handle, &editor, text);
        run(cx, handle, &editor, RunKind::All);
        idle(cx, &editor);
        cx.run_until_parked();
        assert_eq!(*changes.borrow(), [ids[0].clone()], "{text}");
    };

    // A script publishes once, including when a later statement fails after a committed write.
    run_once(
        cx,
        "UPDATE t SET name = 'first'; UPDATE t SET name = 'second'",
    );
    assert_eq!(
        sql(&first_file, "SELECT name FROM t"),
        [vec![Some("second".to_string())]]
    );
    run_once(
        cx,
        "UPDATE t SET name = 'partial'; SELECT * FROM missing; UPDATE t SET name = 'skipped'",
    );
    assert_eq!(
        sql(&first_file, "SELECT name FROM t"),
        [vec![Some("partial".to_string())]]
    );
    cx.update(|cx| {
        let results = &editor.read(cx).results.rows;
        assert_eq!(results.len(), 2);
        assert!(results[1].error);
    });

    // Transaction completion needs its own notification; other sessions see only committed data.
    run_once(cx, "BEGIN; UPDATE t SET name = 'committed'");
    assert_eq!(
        sql(&first_file, "SELECT name FROM t"),
        [vec![Some("partial".to_string())]]
    );
    run_once(cx, "COMMIT");
    assert_eq!(
        sql(&first_file, "SELECT name FROM t"),
        [vec![Some("committed".to_string())]]
    );
    run_once(cx, "BEGIN; UPDATE t SET name = 'rolled back'");
    run_once(cx, "ROLLBACK");
    assert_eq!(
        sql(&first_file, "SELECT name FROM t"),
        [vec![Some("committed".to_string())]]
    );

    changes.borrow_mut().clear();
    set_text(cx, handle, &editor, "SELECT * FROM t");
    run(cx, handle, &editor, RunKind::Explain);
    idle(cx, &editor);
    cx.run_until_parked();
    assert!(
        changes.borrow().is_empty(),
        "Explain does not execute the SQL"
    );

    // Changing the toolbar while SQL is executing must not redirect its completion event.
    set_text(
        cx,
        handle,
        &editor,
        "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n",
    );
    run(cx, handle, &editor, RunKind::All);
    wait(cx, "the original query to execute", |cx| {
        cx.update(|cx| {
            editor
                .read(cx)
                .live
                .as_ref()
                .is_some_and(|live| live.session.try_lock().is_err())
        })
    });
    let cancel = cx.update(|cx| {
        editor
            .read(cx)
            .live
            .as_ref()
            .unwrap()
            .cancel
            .clone()
            .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |editor, cx| {
            editor.pick_connection(ids[1].clone(), window, cx)
        });
    })
    .unwrap();
    runtime::runtime().block_on(cancel()).unwrap();
    idle(cx, &editor);
    cx.run_until_parked();
    assert_eq!(*changes.borrow(), [ids[0].clone()]);
    cx.update(|cx| {
        assert_eq!(
            editor.read(cx).target.connection.as_deref(),
            Some(ids[1].as_str())
        );
        assert!(editor.read(cx).results.rows.last().unwrap().error);
    });
}

#[gpui_kit::test]
fn accepted_results_keep_their_rows_and_original_source(cx: &mut TestAppContext) {
    let (_home, home) = isolate_home("result-source");
    let first_file = home.join("first.db");
    let second_file = home.join("second.db");
    sql(
        &first_file,
        "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t VALUES (1, 'a'), (2, 'b')",
    );
    sql(
        &second_file,
        "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t VALUES (1, 'other database')",
    );
    cx.executor().allow_parking();
    let ids = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
        Settings::update(cx, |s| s.sync_record = false);
        ConnectionStore::global(cx).update(cx, |s, cx| {
            s.set_key(Key::random());
            [&first_file, &second_file]
                .into_iter()
                .enumerate()
                .map(|(ix, file)| {
                    let mut connection = Connection::new("sqlite");
                    connection.name = format!("source-{ix}");
                    connection.general.file = Some(file.to_string_lossy().into_owned());
                    connection.general.database = Some("main".into());
                    let id = connection.id.clone();
                    s.upsert(connection, &[], cx).unwrap();
                    id
                })
                .collect::<Vec<_>>()
        })
    });
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |s, cx| {
            for id in &ids {
                s.open(id.clone(), Secrets::default(), cx);
            }
        })
    });
    wait(cx, "both source connections", |cx| {
        cx.update(|cx| {
            ids.iter()
                .all(|id| Sessions::global(cx).read(cx).state(id).is_open())
        })
    });
    let (handle, editor) = cx.update(|cx| {
        let mut editor = None;
        let handle = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let view = QueryEditor::new(
                QuerySource::Untitled(1),
                Target {
                    connection: Some(ids[0].clone()),
                    database: Some("main".into()),
                    schema: None,
                },
                "SELECT id, name FROM t ORDER BY id".into(),
                window,
                cx,
            );
            editor = Some(view.clone());
            cx.new(|_| Host { editor: view })
        })
        .unwrap();
        (handle.0, editor.unwrap())
    });
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    let view = cx.update(|cx| {
        editor.update(cx, |e, cx| {
            let tab = e.results.tabs[0].id;
            e.results.toggle_pin(tab, cx);
            let Body::Grid(view) = &e.results.tabs[0].body else {
                panic!("grid result")
            };
            view.clone()
        })
    });
    wait(cx, "editable original result", |cx| {
        cx.update(|cx| view.read(cx).editable())
    });
    // Exercise DataTable's actual header hit target and cached arrow state, not request_sort.
    let click_name_sort = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.click(gpui_kit::ElementId::NamedInteger("icon-sort".into(), 2), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    click_name_sort(cx); // Default -> descending.
    click_name_sort(cx); // Descending -> ascending (the last accepted sort).
    for blank_draft in [false, true] {
        cx.update(|cx| {
            view.read(cx).grid.clone().update(cx, |g, cx| {
                g.update_model(cx, |m| {
                    if blank_draft {
                        m.insert_row();
                    } else {
                        m.set_value(0, 1, Some("pending sort".into()));
                    }
                });
            });
        });
        let before = cx.update(|cx| view.read(cx).rows(cx));
        click_name_sort(cx); // Rejected: the accepted ascending arrow must remain.
        cx.update(|cx| {
            assert_eq!(view.read(cx).rows(cx), before);
            assert_eq!(
                view.read(cx).status(),
                rust_i18n::t!("query.sort_blocked").as_ref()
            );
            view.read(cx).grid.clone().update(cx, |g, cx| {
                g.update_model(cx, |m| m.discard());
            });
        });
        click_name_sort(cx); // Ascending -> default: retain the accepted row order.
        cx.update(|cx| assert_eq!(view.read(cx).rows(cx)[0][1].as_deref(), Some("a")));
        click_name_sort(cx); // Default -> descending; catches a stale internal header arrow.
        cx.update(|cx| assert_eq!(view.read(cx).rows(cx)[0][1].as_deref(), Some("b")));
        click_name_sort(cx); // Descending -> ascending for the next case.
    }

    // A clean in-flight reload accepts sorting and reapplies it to the returned rows.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.refresh_now(window, cx);
            assert!(v.is_busy());
            v.sort_now(Some(("name".into(), false)), window, cx);
            assert!(v.is_busy());
            assert_eq!(v.rows(cx)[0][1].as_deref(), Some("b"));
        });
    })
    .unwrap();
    wait(cx, "sorting during a clean reload", |cx| {
        cx.update(|cx| !view.read(cx).is_busy())
    });
    cx.update(|cx| assert_eq!(view.read(cx).rows(cx)[0][1].as_deref(), Some("b")));
    click_name_sort(cx); // Return to ascending before checking pending input.

    // The internal refresh guard must settle active input without issuing SQL.
    let revision = navidog_db::log::revision();
    cx.update_window(handle, |_, window, cx| {
        view.read(cx)
            .grid
            .clone()
            .update(cx, |g, cx| g.start_edit((0, 1), window, cx));
        window.render_frame(cx);
        window.press("secondary-a", cx);
        window.input("uncommitted refresh", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.refresh_now(window, cx));
        assert!(!view.read(cx).is_busy());
        assert_eq!(
            view.read(cx).rows(cx)[0][1].as_deref(),
            Some("uncommitted refresh")
        );
        assert!(view.read(cx).grid.read(cx).model(cx).has_changes());
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        navidog_db::log::since(revision)
            .0
            .iter()
            .all(|entry| { entry.sql != "SELECT id, name FROM t ORDER BY id" })
    );
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.grid.update(cx, |g, cx| {
                g.update_model(cx, |m| {
                    m.discard();
                    m.insert_row();
                })
            });
            v.refresh_now(window, cx);
            assert!(!v.is_busy());
            assert_eq!(v.rows(cx).len(), 3);
            assert!(v.grid.read(cx).model(cx).change_set().inserts.is_empty());
            v.grid
                .update(cx, |g, cx| g.update_model(cx, |m| m.discard()));
        });
    })
    .unwrap();

    // Reopening drops only the physical editor session, keeping the document and result drafts.
    let unsaved_sql = "SELECT id, name FROM t ORDER BY id\n-- unsaved query survives reopening";
    cx.update_window(handle, |_, window, cx| {
        let state = editor.read(cx).editor.clone();
        state.update(cx, |state, cx| {
            let end = state.value().len();
            state.replace_text_in_range(Some(0..end), unsaved_sql, window, cx);
        });
    })
    .unwrap();
    cx.update(|cx| {
        view.read(cx).grid.clone().update(cx, |g, cx| {
            g.update_model(cx, |m| {
                m.set_value(0, 1, Some("draft survives reopening".into()));
                m.insert_row();
            });
        })
    });
    cx.run_until_parked();
    let draft_rows = cx.update(|cx| view.read(cx).rows(cx));
    cx.update(|cx| {
        assert!(editor.read(cx).live.is_some());
        assert!(editor.read(cx).dirty);
        Sessions::global(cx).update(cx, |sessions, cx| sessions.close_for_reopen(&ids[0], cx));
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(editor.read(cx).live.is_none());
        assert_eq!(
            editor.read(cx).editor.read(cx).value().to_string(),
            unsaved_sql
        );
        assert!(editor.read(cx).dirty);
        assert_eq!(view.read(cx).rows(cx), draft_rows);
        assert!(view.read(cx).grid.read(cx).model(cx).has_changes());
        assert!(
            view.read(cx)
                .grid
                .read(cx)
                .model(cx)
                .change_set()
                .inserts
                .is_empty()
        );
        assert!(editor.read(cx).results.tabs.iter().any(|tab| {
            tab.pinned && matches!(&tab.body, Body::Grid(result) if *result == view)
        }));
    });
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |sessions, cx| {
            sessions.open(ids[0].clone(), Secrets::default(), cx);
        })
    });
    wait(cx, "the original connection to reopen", |cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&ids[0]).is_open())
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            editor.read(cx).live.is_none(),
            "a user run lazily creates the next editor session"
        );
        assert_eq!(
            editor.read(cx).editor.read(cx).value().to_string(),
            unsaved_sql
        );
        assert!(editor.read(cx).dirty);
        assert_eq!(view.read(cx).rows(cx), draft_rows);
        assert!(view.read(cx).grid.read(cx).model(cx).has_changes());
        view.read(cx)
            .grid
            .clone()
            .update(cx, |g, cx| g.update_model(cx, |m| m.discard()));
    });
    assert_eq!(
        sql(&first_file, "SELECT id, name FROM t ORDER BY id"),
        [
            vec![Some("1".into()), Some("a".into())],
            vec![Some("2".into()), Some("b".into())],
        ]
    );
    set_text(cx, handle, &editor, "SELECT id, name FROM t ORDER BY id");

    // A metadata caller can leave a transaction open; result Apply must use its own session.
    let metadata_transaction = cx.update(|cx| {
        Sessions::global(cx)
            .read(cx)
            .run_rel(&ids[0], Some("main"), |rel| {
                Box::pin(async move { rel.execute("BEGIN").await.map(|_| ()) })
            })
            .unwrap()
    });
    runtime::runtime()
        .block_on(metadata_transaction)
        .unwrap()
        .unwrap();

    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.grid.update(cx, |g, cx| {
                g.update_model(cx, |m| {
                    m.set_value(0, 1, Some("z".into()));
                    m.select((1, 0), false);
                    m.delete_selected_rows();
                    let row = m.insert_row();
                    m.set_value(row, 0, Some("3".into()));
                    m.set_value(row, 1, Some("c".into()));
                })
            });
            v.commit_now(window, cx);
        })
    })
    .unwrap();
    wait(cx, "accepted update, insert and delete", |cx| {
        cx.update(|cx| !view.read(cx).is_busy())
    });
    let metadata_commit = cx.update(|cx| {
        Sessions::global(cx)
            .read(cx)
            .run_rel(&ids[0], Some("main"), |rel| {
                Box::pin(async move { rel.execute("COMMIT").await.map(|_| ()) })
            })
            .unwrap()
    });
    runtime::runtime()
        .block_on(metadata_commit)
        .unwrap()
        .unwrap();
    assert_eq!(
        sql(&first_file, "SELECT id, name FROM t ORDER BY id"),
        [
            vec![Some("1".into()), Some("z".into())],
            vec![Some("3".into()), Some("c".into())]
        ]
    );
    cx.update(|cx| {
        view.read(cx)
            .grid
            .clone()
            .update(cx, |g, cx| g.request_sort(Some(1), true, cx))
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let grid = view.read(cx).grid.clone();
        grid.update(cx, |g, cx| {
            g.set_hidden_columns(&["id".into()].into(), cx);
            g.set_hidden_columns(&Default::default(), cx);
        });
        assert_eq!(
            view.read(cx).rows(cx),
            [
                vec![Some("3".into()), Some("c".into())],
                vec![Some("1".into()), Some("z".into())]
            ]
        );
    });

    // The fixed result keeps its source after the editor opens a different connection.
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| e.pick_connection(ids[1].clone(), window, cx))
    })
    .unwrap();
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    cx.update(|cx| {
        assert!(editor.read(cx).results.tabs.iter().any(|tab| tab.pinned));
        Settings::update(cx, |s| s.sync_record = true);
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.grid.update(cx, |g, cx| {
                g.update_model(cx, |m| m.set_value(0, 1, Some("source A".into())))
            });
            v.commit_now(window, cx);
        })
    })
    .unwrap();
    wait(cx, "refreshing the original source", |cx| {
        cx.update(|cx| !view.read(cx).is_busy())
    });
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).rows(cx),
            [
                vec![Some("3".into()), Some("source A".into())],
                vec![Some("1".into()), Some("z".into())]
            ]
        )
    });
    assert_eq!(
        sql(&second_file, "SELECT name FROM t"),
        [vec![Some("other database".into())]]
    );
    cx.update(|cx| {
        editor.update(cx, |e, _| {
            let tab = e
                .results
                .tabs
                .iter()
                .find(|tab| matches!(&tab.body, Body::Grid(result) if *result == view))
                .unwrap();
            e.results.page = crate::results::Page::Tab(tab.id);
        })
    });
    click_name_sort(cx); // Reload kept ascending: the next click clears it.
    cx.update(|cx| assert_eq!(view.read(cx).rows(cx)[0][0].as_deref(), Some("3")));
    click_name_sort(cx); // Default -> descending, in both the indicator and row data.
    cx.update(|cx| assert_eq!(view.read(cx).rows(cx)[0][0].as_deref(), Some("1")));
    click_name_sort(cx); // Restore ascending before continuing.

    // A matching editor session keeps its temporary objects available during refresh.
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| e.pick_connection(ids[0].clone(), window, cx))
    })
    .unwrap();
    set_text(
        cx,
        handle,
        &editor,
        "CREATE TEMP VIEW session_view AS SELECT id,name FROM t; SELECT id,name FROM session_view ORDER BY id",
    );
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    let temporary = cx.update(|cx| {
        let Body::Grid(view) = &editor.read(cx).results.tabs.last().unwrap().body else {
            panic!("temporary view result")
        };
        view.clone()
    });
    wait(cx, "editable temporary view result", |cx| {
        cx.update(|cx| temporary.read(cx).editable())
    });
    cx.update_window(handle, |_, window, cx| {
        temporary.update(cx, |v, cx| {
            v.grid.update(cx, |g, cx| {
                g.update_model(cx, |m| m.set_value(0, 1, Some("temporary context".into())))
            });
            v.commit_now(window, cx);
        })
    })
    .unwrap();
    wait(cx, "refreshing within the matching session", |cx| {
        cx.update(|cx| !temporary.read(cx).is_busy())
    });
    cx.update(|cx| {
        assert_eq!(
            temporary.read(cx).rows(cx)[0][1],
            Some("temporary context".into())
        )
    });

    // Changing the selected target while a statement runs cannot relabel that run's source.
    set_text(cx, handle, &editor, "SELECT id,name FROM t");
    run(cx, handle, &editor, RunKind::All);
    cx.update(|cx| editor.update(cx, |e, cx| e.pick_database("changed-selection".into(), cx)));
    idle(cx, &editor);
    cx.update(|cx| {
        let source = &editor.read(cx).results.tabs.last().unwrap().info;
        assert_eq!(source.connection_id, ids[0]);
        assert_eq!(source.database.as_deref(), Some("main"));
    });

    // A committed batch stays accepted if synchronizing the result subsequently fails.
    sql(
        &first_file,
        "CREATE TABLE applied_count(n INTEGER); INSERT INTO applied_count VALUES(0); CREATE TRIGGER count_apply AFTER UPDATE ON t BEGIN UPDATE applied_count SET n = n + 1; END",
    );
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.sql = "SELECT * FROM missing_refresh_target".into();
            v.grid.update(cx, |g, cx| {
                g.update_model(cx, |m| {
                    m.set_value(0, 1, Some("accepted once".into()));
                    m.insert_row();
                    m.insert_row();
                })
            });
            v.commit_now(window, cx);
        });
    })
    .unwrap();
    wait(cx, "the failed post-commit refresh", |cx| {
        cx.update(|cx| !view.read(cx).is_busy())
    });
    cx.update(|cx| {
        let model = view.read(cx).grid.read(cx).model(cx);
        assert!(
            model.has_changes(),
            "unsubmitted blank drafts survive a failed reload"
        );
        let changes = model.change_set();
        assert!(
            changes.updates.is_empty() && changes.inserts.is_empty() && changes.deletes.is_empty()
        );
        assert_eq!(model.row_count(), 4);
        assert!(model.value(2, 0).is_none() && model.value(3, 0).is_none());
        assert_eq!(
            view.read(cx).rows(cx)[0][1].as_deref(),
            Some("accepted once")
        );
        assert!(view.read(cx).status().contains("missing_refresh_target"));
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.commit_now(window, cx));
        assert!(!view.read(cx).is_busy());
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        sql(&first_file, "SELECT n FROM applied_count"),
        [vec![Some("1".into())]]
    );

    // Blank drafts also survive acceptance without synchronization and a successful reload.
    for (sync, value) in [
        (false, "accepted without sync"),
        (true, "accepted with sync"),
    ] {
        cx.update(|cx| Settings::update(cx, |settings| settings.sync_record = sync));
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |v, cx| {
                v.sql = "SELECT id, name FROM t ORDER BY id".into();
                v.grid.update(cx, |grid, cx| {
                    grid.update_model(cx, |model| model.set_value(0, 1, Some(value.into())));
                });
                v.commit_now(window, cx);
            });
        })
        .unwrap();
        wait(cx, "committing while retaining blank drafts", |cx| {
            cx.update(|cx| !view.read(cx).is_busy())
        });
        cx.update(|cx| {
            let model = view.read(cx).grid.read(cx).model(cx);
            assert_eq!(
                model.row_count(),
                4,
                "synchronization must not duplicate drafts"
            );
            let changes = model.change_set();
            assert!(
                changes.updates.is_empty()
                    && changes.inserts.is_empty()
                    && changes.deletes.is_empty()
            );
            assert!(model.value(2, 0).is_none() && model.value(3, 0).is_none());
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |v, cx| v.commit_now(window, cx))
        })
        .unwrap();
    }
    assert_eq!(
        sql(&first_file, "SELECT n FROM applied_count"),
        [vec![Some("3".into())]]
    );
    cx.update(|cx| {
        view.read(cx)
            .grid
            .clone()
            .update(cx, |g, cx| g.update_model(cx, |model| model.discard()))
    });

    // Retiring a connection in the same UI update fences any old completion, even if the
    // database already finished the write. No old success may accept the new view's draft.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.grid.update(cx, |g, cx| {
                g.update_model(cx, |model| {
                    model.set_value(0, 1, Some("retired operation".into()))
                })
            });
            v.commit_now(window, cx);
            assert!(v.is_busy());
        });
        Sessions::global(cx).update(cx, |sessions, cx| {
            sessions.close_for_reopen(&ids[0], cx);
            sessions.open(ids[0].clone(), Secrets::default(), cx);
        });
    })
    .unwrap();
    wait(
        cx,
        "the replacement session and retired result callback",
        |cx| {
            cx.update(|cx| {
                Sessions::global(cx).read(cx).state(&ids[0]).is_open() && !view.read(cx).is_busy()
            })
        },
    );
    cx.update(|cx| {
        let result = view.read(cx);
        let model = result.grid.read(cx).model(cx);
        assert!(model.has_changes());
        assert_eq!(
            model.value(0, 1).and_then(|cell| cell.as_deref()),
            Some("retired operation")
        );
        assert_eq!(
            result.status(),
            rust_i18n::t!("query.connection_changed_during_operation").as_ref()
        );
        assert_ne!(result.status(), "query.connection_changed_during_operation");
        result
            .grid
            .clone()
            .update(cx, |g, cx| g.update_model(cx, |model| model.discard()));
    });

    // A changed result shape invalidates the accepted sort rather than retaining a stale arrow.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.sql = "SELECT id FROM t ORDER BY id".into();
            v.refresh_now(window, cx);
        });
    })
    .unwrap();
    wait(cx, "refreshing changed columns", |cx| {
        cx.update(|cx| !view.read(cx).is_busy())
    });
    cx.update(|cx| {
        editor.update(cx, |e, _| {
            let tab = e
                .results
                .tabs
                .iter()
                .find(|tab| matches!(&tab.body, Body::Grid(result) if *result == view))
                .unwrap();
            e.results.page = crate::results::Page::Tab(tab.id);
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        window.click(gpui_kit::ElementId::NamedInteger("icon-sort".into(), 1), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).rows(cx)[0][0].as_deref(), Some("3")));
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
        assert_eq!(view.read(cx).rows(cx), [vec![Some("22".into())]]);
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
        assert_eq!(view.read(cx).rows(cx), [vec![Some("33".into())]]);
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
        let rows = view.read(cx).rows(cx);
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
    let changes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(
            &Workspace::global(cx),
            move |_, event: &WorkspaceEvent, _| {
                if let WorkspaceEvent::DatabaseChanged(connection) = event {
                    observed.borrow_mut().push(connection.clone());
                }
            },
        )
    });

    // An incomplete statement only gets a new line; with the terminator it runs
    let type_and_enter = |cx: &mut TestAppContext, text: &str| {
        enter_console(cx, handle, &console, text);
    };
    type_and_enter(cx, "SELECT v FROM c");
    assert_eq!(
        cx.update(|cx| console.read(cx).input_state().read(cx).value().to_string()),
        "SELECT v FROM c\n"
    );
    assert!(
        changes.borrow().is_empty(),
        "incomplete input executes no SQL"
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
    cx.run_until_parked();
    assert_eq!(changes.borrow().as_slice(), std::slice::from_ref(&id));

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
    cx.run_until_parked();
    assert_eq!(*changes.borrow(), [id.clone(), id.clone()]);
    type_and_enter(cx, "SELECT 42 AS resumed;");
    wait(cx, "the console to run after cancellation", |cx| {
        cx.update(|cx| console.read(cx).output_text().contains("| 42 "))
    });
    cx.run_until_parked();
    assert_eq!(*changes.borrow(), [id.clone(), id.clone(), id.clone()]);

    // The driver can lose earlier successful results when a later statement fails.
    type_and_enter(
        cx,
        "INSERT INTO c(v) VALUES ('partial'); SELECT * FROM missing;",
    );
    wait(cx, "the partially successful console input", |cx| {
        cx.update(|cx| {
            console
                .read(cx)
                .output_text()
                .contains("no such table: missing")
        })
    });
    cx.run_until_parked();
    assert_eq!(*changes.borrow(), [id.clone(), id.clone(), id.clone(), id]);
    assert_eq!(
        sql(&file, "SELECT v FROM c WHERE id = 2"),
        [vec![Some("partial".to_string())]]
    );

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
                assert_eq!(
                    view.read(cx).rows(cx),
                    [vec![Some(value.into())]],
                    "{driver}"
                );
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
                assert_eq!(view.read(cx).rows(cx), [vec![Some("7".into())]]);
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

        // SQL-driven context is independent of the toolbar, and a pinned refresh restores it.
        let suffix = std::process::id();
        let first = format!("navidog_source_a_{suffix}");
        let second = format!("navidog_source_b_{suffix}");
        let table = format!("navidog_source_rows_{suffix}");
        let setup = if driver == "postgres" {
            format!(
                "DROP SCHEMA IF EXISTS {first} CASCADE; DROP SCHEMA IF EXISTS {second} CASCADE; \
                 CREATE SCHEMA {first}; CREATE SCHEMA {second}; \
                 CREATE TABLE {first}.{table}(id INTEGER PRIMARY KEY, name TEXT); \
                 INSERT INTO {first}.{table} VALUES(1, 'original source')"
            )
        } else {
            format!(
                "DROP DATABASE IF EXISTS {second}; CREATE DATABASE {second}; \
                 CREATE TABLE {second}.{table}(id INTEGER PRIMARY KEY, name VARCHAR(100)); \
                 INSERT INTO {second}.{table} VALUES(1, 'original source')"
            )
        };
        live_sql(driver, port, database, &setup);
        let change = if driver == "postgres" {
            format!("SET search_path TO {second}, {first}")
        } else {
            format!("USE {second}")
        };
        set_text(
            cx,
            handle,
            &editor,
            &format!("{change}; SELECT id,name FROM {table}"),
        );
        run(cx, handle, &editor, RunKind::All);
        idle(cx, &editor);
        let original = cx.update(|cx| {
            editor.update(cx, |e, cx| {
                assert!(e.results.rows.iter().all(|row| !row.error), "{driver}");
                let tab = e.results.tabs.last().unwrap();
                assert_eq!(
                    tab.info.database.as_deref(),
                    Some(if driver == "postgres" {
                        database
                    } else {
                        &second
                    })
                );
                if driver == "postgres" {
                    assert_eq!(tab.info.schema.as_deref(), Some(second.as_str()));
                    assert!(tab.info.search_path.as_ref().unwrap().contains(&first));
                }
                assert_eq!(e.target.database.as_deref(), Some(database));
                let id = tab.id;
                let Body::Grid(view) = &tab.body else {
                    panic!("source result")
                };
                let view = view.clone();
                e.results.toggle_pin(id, cx);
                view
            })
        });
        wait(cx, "the SQL-context result to become editable", |cx| {
            cx.update(|cx| original.read(cx).editable())
        });

        // An unchanged toolbar keeps the SQL context across runs.
        set_text(cx, handle, &editor, &format!("SELECT id,name FROM {table}"));
        run(cx, handle, &editor, RunKind::All);
        idle(cx, &editor);
        cx.update(|cx| {
            let tab = editor.read(cx).results.tabs.last().unwrap();
            assert_eq!(
                tab.info.database.as_deref(),
                Some(if driver == "postgres" {
                    database
                } else {
                    &second
                })
            );
            assert!(editor.read(cx).results.rows.iter().all(|row| !row.error));
        });

        // Change the live session so committing the old result must open an independent source.
        cx.update(|cx| {
            editor.update(cx, |e, cx| {
                if driver == "postgres" {
                    e.pick_schema(first.clone(), cx);
                } else {
                    e.pick_database("mysql".into(), cx);
                }
                Settings::update(cx, |s| s.sync_record = true);
            })
        });
        set_text(cx, handle, &editor, "SELECT 1");
        run(cx, handle, &editor, RunKind::All);
        idle(cx, &editor);
        cx.update_window(handle, |_, window, cx| {
            original.update(cx, |view, cx| {
                view.grid.update(cx, |grid, cx| {
                    grid.update_model(cx, |model| {
                        model.set_value(0, 1, Some("refreshed original".into()))
                    })
                });
                view.commit_now(window, cx);
            });
        })
        .unwrap();
        wait(cx, "the original SQL context to refresh", |cx| {
            cx.update(|cx| !original.read(cx).is_busy())
        });
        cx.update(|cx| {
            assert!(
                !original.read(cx).grid.read(cx).model(cx).has_changes(),
                "{driver}: the refreshed values must come from the server, not the edit overlay"
            );
            assert_eq!(
                original.read(cx).rows(cx),
                [vec![Some("1".into()), Some("refreshed original".into())]],
                "{driver}"
            );
        });
        let cleanup = if driver == "postgres" {
            format!("DROP SCHEMA {second} CASCADE; DROP SCHEMA {first} CASCADE")
        } else {
            format!("DROP DATABASE {second}")
        };
        live_sql(driver, port, database, &cleanup);
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

#[gpui_kit::test]
fn a_dirty_result_keeps_its_sort_arrow(cx: &mut TestAppContext) {
    let (_home, home) = isolate_home("sort-arrow");
    let file = home.join("sort.db");
    sql(
        &file,
        "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t VALUES (1, 'a'), (2, 'c'), (3, 'b')",
    );
    cx.executor().allow_parking();
    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        crate::init(cx);
        Settings::update(cx, |s| s.sync_record = false);
        ConnectionStore::global(cx).update(cx, |s, cx| {
            s.set_key(Key::random());
            let mut connection = Connection::new("sqlite");
            connection.name = "sort".into();
            connection.general.file = Some(file.to_string_lossy().into_owned());
            connection.general.database = Some("main".into());
            let id = connection.id.clone();
            s.upsert(connection, &[], cx).unwrap();
            id
        })
    });
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |s, cx| s.open(id.clone(), Secrets::default(), cx))
    });
    wait(cx, "the sort connection", |cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });
    let (handle, editor) = cx.update(|cx| {
        let mut editor = None;
        // raw-px: tall enough that the result header is in the frame
        let test_size = size(gpui_kit::px(1000.), gpui_kit::px(800.));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                origin: Default::default(),
                size: test_size,
            })),
            ..Default::default()
        };
        let handle = gpui_kit::open_window(options, cx, |window, cx| {
            let view = QueryEditor::new(
                QuerySource::Untitled(1),
                Target {
                    connection: Some(id.clone()),
                    database: Some("main".into()),
                    schema: None,
                },
                "SELECT id, name FROM t ORDER BY id".into(),
                window,
                cx,
            );
            editor = Some(view.clone());
            cx.new(|_| Host { editor: view })
        })
        .unwrap();
        (handle.0, editor.unwrap())
    });
    run(cx, handle, &editor, RunKind::All);
    idle(cx, &editor);
    let view = cx.update(|cx| {
        editor.update(cx, |e, _| {
            let Body::Grid(view) = &e.results.tabs[0].body else {
                panic!("grid result")
            };
            view.clone()
        })
    });
    wait(cx, "editable sort result", |cx| {
        cx.update(|cx| view.read(cx).editable())
    });
    let click_sort = |cx: &mut TestAppContext, col: u64| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.click(
                gpui_kit::ElementId::NamedInteger("icon-sort".into(), col),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
    };
    // Column 0 is the row-number marker. 2 is `name`; the first click sorts descending.
    click_sort(cx, 2);
    let sorted = [
        vec![Some("2".into()), Some("c".into())],
        vec![Some("3".into()), Some("b".into())],
        vec![Some("1".into()), Some("a".into())],
    ];
    cx.update(|cx| {
        assert_eq!(view.read(cx).rows(cx), sorted);
        let grid = view.read(cx).grid.clone();
        assert_eq!(
            grid.read(cx)
                .header_sort(cx)
                .map(|(name, asc)| (name.to_string(), asc)),
            Some(("name".into(), false))
        );
        grid.update(cx, |g, cx| {
            g.update_model(cx, |m| m.set_value(0, 1, Some("z".into())))
        });
    });
    click_sort(cx, 1);
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).rows(cx),
            [
                vec![Some("2".into()), Some("z".into())],
                vec![Some("3".into()), Some("b".into())],
                vec![Some("1".into()), Some("a".into())],
            ]
        );
        assert_eq!(
            view.read(cx).status_text(),
            "Commit or discard changes before sorting."
        );
        let grid = view.read(cx).grid.clone();
        assert_eq!(
            grid.read(cx)
                .header_sort(cx)
                .map(|(name, asc)| (name.to_string(), asc)),
            Some(("name".into(), false))
        );
        grid.update(cx, |g, cx| {
            g.update_model(cx, |m| {
                m.discard();
                m.insert_row();
            })
        });
    });
    click_sort(cx, 1);
    cx.update(|cx| {
        let mut expected = sorted.to_vec();
        expected.push(vec![None, None]);
        assert_eq!(view.read(cx).rows(cx), expected);
        assert_eq!(
            view.read(cx).status_text(),
            "Commit or discard changes before sorting."
        );
        let grid = view.read(cx).grid.clone();
        assert_eq!(
            grid.read(cx)
                .header_sort(cx)
                .map(|(name, asc)| (name.to_string(), asc)),
            Some(("name".into(), false))
        );
    });
}
