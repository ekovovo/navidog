//! Renders the main window to PNGs with the headless Metal renderer for side-by-side review
//! (no pixel assertions). Run `cargo test -p navidog --test snapshot`; output goes to `target/snapshots/`.
//! AppKit requires the main thread, hence `harness = false`.

fn main() {
    #[cfg(target_os = "macos")]
    macos::run();
    #[cfg(not(target_os = "macos"))]
    println!("snapshot: 跳过，只有 macOS 提供 headless 渲染器");
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::Arc;

    use gpui_kit::component::{Theme, ThemeMode, WindowExt as _};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Focusable as _, HeadlessAppContext, InputEvent as _};
    use navidog::main_window::MainWindow;
    use navidog::state::AppState;
    use navidog_core::actions::{NewConnection, NewConnectionOf, OpenObject};
    use navidog_core::connection::Connection;
    use navidog_core::crypto::Key;
    use navidog_core::object::{ObjectKind, ObjectRef, Scope};
    use navidog_core::settings::Settings;
    use navidog_core::store::ConnectionStore;
    use navidog_core::workspace::{Selection, Workspace};
    use navidog_db::sessions::Sessions;
    use navidog_ui::NavidogAssets;

    pub fn run() {
        let home = std::env::temp_dir().join(format!("navidog-snapshot-{}", std::process::id()));
        // SAFETY: single-threaded startup; no thread reading the environment exists yet
        unsafe {
            std::env::set_var("NAVIDOG_HOME", &home);
            std::env::set_var("LANG", "zh_CN.UTF-8");
        }
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
        std::fs::create_dir_all(&out).unwrap();

        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(NavidogAssets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            navidog::init(cx);
            seed(cx);
        });
        let (handle, view) = cx
            .update(|cx| {
                let mut options = navidog::main_window_options(&AppState::default(), cx);
                options.show = false;
                options.focus = false;
                gpui_kit::open_window(options, cx, |window, cx| {
                    cx.new(|cx| MainWindow::new(AppState::default(), window, cx))
                })
            })
            .unwrap();

        let shot_of =
            |cx: &mut HeadlessAppContext, handle: gpui_kit::AnyWindowHandle, name: &str| {
                // Icons and other assets load asynchronously; let the tasks finish before rendering
                for _ in 0..3 {
                    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                        .unwrap();
                    cx.run_until_parked();
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                let image = cx
                    .capture_screenshot(handle)
                    .expect("needs the Metal renderer");
                image.save(out.join(name)).unwrap();
                println!("saved {}", out.join(name).display());
            };
        let shot = |cx: &mut HeadlessAppContext, name: &str| shot_of(cx, handle, name);
        for (mode, name) in [
            (ThemeMode::Dark, "main-dark.png"),
            (ThemeMode::Light, "main-light.png"),
        ] {
            cx.update(|cx| Theme::change(mode, None, cx));
            shot(&mut cx, name);
        }
        let dispatch = |cx: &mut HeadlessAppContext, action: Box<dyn gpui_kit::Action>| {
            cx.update_window(handle, |_, window, cx| {
                window.focus(&view.focus_handle(cx), cx);
                window.dispatch_action(action, cx);
            })
            .unwrap();
        };
        dispatch(&mut cx, Box::new(NewConnection));
        shot(&mut cx, "picker.png");
        cx.update_window(handle, |_, window, cx| window.close_all_dialogs(cx))
            .unwrap();
        for driver in ["mysql", "postgres", "sqlite"] {
            dispatch(
                &mut cx,
                Box::new(NewConnectionOf {
                    driver: driver.into(),
                }),
            );
            shot(&mut cx, &format!("form-{driver}.png"));
            cx.update_window(handle, |_, window, cx| window.close_all_dialogs(cx))
                .unwrap();
        }

        // M2: open the sample SQLite database, expand to objects, show the object list and editor tabs
        let lite = cx.update(|cx| {
            ConnectionStore::global(cx)
                .read(cx)
                .connections()
                .iter()
                .find(|c| c.driver == "sqlite")
                .unwrap()
                .id
                .clone()
        });
        cx.update(|cx| {
            Workspace::global(cx).update(cx, |w, cx| {
                w.select(Selection::Connection(lite.clone()), cx)
            });
            Sessions::global(cx).update(cx, |s, cx| s.open(lite.clone(), Default::default(), cx));
        });
        wait(&mut cx, |cx| {
            Sessions::global(cx).read(cx).state(&lite).is_open()
        });
        let scope = Scope::new(lite.clone(), "main", None);
        cx.update(|cx| {
            Workspace::global(cx).update(cx, |w, cx| {
                w.select(Selection::Category(scope.clone(), ObjectKind::Table), cx)
            })
        });
        wait(&mut cx, |cx| {
            matches!(
                navidog_navigator::catalog::Catalog::global(cx)
                    .read(cx)
                    .objects(&scope, ObjectKind::Table),
                Some(navidog_navigator::catalog::Load::Ready(_))
            )
        });
        let actor = ObjectRef {
            scope: scope.clone(),
            kind: ObjectKind::Table,
            name: "actor".into(),
        };
        cx.update(|cx| {
            Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Object(actor.clone()), cx))
        });
        shot(&mut cx, "objects.png");
        let view = ObjectRef {
            scope,
            kind: ObjectKind::View,
            name: "actor_names".into(),
        };
        dispatch(&mut cx, Box::new(OpenObject { object: view }));
        wait(&mut cx, |_| false);
        shot(&mut cx, "ddl-tab.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "ddl-tab-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        dispatch(
            &mut cx,
            Box::new(OpenObject {
                object: actor.clone(),
            }),
        );
        wait(&mut cx, |_| false);
        shot(&mut cx, "viewer.png");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("viewer-find", cx);
            window.dispatch_action(Box::new(navidog_grid::ShowReplace), cx);
        })
        .unwrap();
        shot(&mut cx, "viewer-find.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "viewer-find-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        let actor_ref = actor.clone();
        let wide = ObjectRef {
            name: "wide".into(),
            ..actor
        };
        dispatch(&mut cx, Box::new(OpenObject { object: wide }));
        wait(&mut cx, |_| false);
        shot(&mut cx, "viewer-wide.png");
        cx.update_window(handle, |_, window, cx| {
            // raw-px: a horizontal scroll across several columns
            let dx = gpui_kit::px(-2000.);
            let delta = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(dx, gpui_kit::Pixels::ZERO));
            window.scroll(
                gpui_kit::ElementId::NamedInteger("row".into(), 0),
                delta,
                cx,
            );
        })
        .unwrap();
        shot(&mut cx, "viewer-wide-scrolled.png");
        cx.update_window(handle, |_, window, cx| {
            // raw-px: far past the last column, so the grid stops at its right edge
            let dx = gpui_kit::px(-100000.);
            let delta = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(dx, gpui_kit::Pixels::ZERO));
            window.scroll(
                gpui_kit::ElementId::NamedInteger("row".into(), 0),
                delta,
                cx,
            );
        })
        .unwrap();
        shot(&mut cx, "viewer-wide-end.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "viewer-wide-end-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));

        // Keep the last batch SQL visible instead of replacing it with a sync SELECT.
        let record_settings = cx.update(|cx| {
            let s = Settings::global(cx);
            let saved = (s.sync_record, s.auto_begin_transaction);
            Settings::update(cx, |s| {
                s.sync_record = false;
                s.auto_begin_transaction = false;
            });
            saved
        });
        cx.update_window(handle, |_, window, cx| {
            // raw-px: return from the last column to the row markers before selecting.
            let dx = gpui_kit::px(100000.);
            window.scroll(
                gpui_kit::ElementId::NamedInteger("row".into(), 0),
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(dx, gpui_kit::Pixels::ZERO)),
                cx,
            );
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click_at(
                gpui_kit::ElementId::NamedInteger("row".into(), 0),
                gpui_kit::point(
                    navidog_ui::tokens::grid::ROW_NUMBER_WIDTH.to_pixels(window.rem_size()) / 2.,
                    window.rem_size() / 2.,
                ),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.press("shift-down", cx))
            .unwrap();
        cx.run_until_parked();
        for button in ["row-delete", "row-apply"] {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(button, cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            let empty = cx
                .update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    window
                        .try_find(gpui_kit::ElementId::NamedInteger("row".into(), 0))
                        .is_none()
                })
                .unwrap();
            if empty {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "batch delete did not commit"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        shot(&mut cx, "viewer-batch-delete-narrow-light.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "viewer-batch-delete-narrow-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        dispatch(&mut cx, Box::new(navidog::actions::ToggleInfoPane));
        shot(&mut cx, "viewer-batch-delete-light.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "viewer-batch-delete-dark.png");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let button = window.find("show-sql").bounds();
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position: gpui_kit::point(
                        button.right() + window.rem_size() / 2.,
                        button.center().y,
                    ),
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        cx.advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        shot(&mut cx, "viewer-batch-delete-hover-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        shot(&mut cx, "viewer-batch-delete-hover-light.png");
        dispatch(&mut cx, Box::new(navidog::actions::ToggleInfoPane));
        cx.update(|cx| {
            Theme::change(ThemeMode::Light, None, cx);
            Settings::update(cx, |s| {
                s.sync_record = record_settings.0;
                s.auto_begin_transaction = record_settings.1;
            });
        });

        // M3: a query editor with results, the explain plan, the console and the history log
        cx.update(|cx| {
            Workspace::global(cx).update(cx, |w, cx| {
                w.select(Selection::Database(lite.clone(), "main".into()), cx)
            })
        });
        dispatch(&mut cx, Box::new(navidog_core::actions::NewQuery));
        cx.run_until_parked();
        let editor = cx
            .update(|cx| navidog_query::active_editor(cx))
            .expect("a query editor");
        cx.update_window(handle, |_, window, cx| {
            editor.update(cx, |e, cx| {
                e.set_text(
                    "-- NAME:演员\nSELECT * FROM actor;\nSELECT COUNT(*) AS films FROM film;\nSELECT * FROM nope;",
                    window,
                    cx,
                );
                e.run_all(window, cx);
            })
        })
        .unwrap();
        wait(&mut cx, |_| false);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            window.dispatch_action(Box::new(navidog_grid::ShowReplace), cx);
        })
        .unwrap();
        shot(&mut cx, "query.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "query-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("query-find-done", cx);
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            editor.update(cx, |e, cx| {
                e.set_text("SELECT a.first_name, f.title FROM actor a JOIN film_actor fa ON fa.actor_id = a.id JOIN film f ON f.id = fa.film_id WHERE a.id = 1", window, cx);
                e.explain_all(window, cx);
            })
        })
        .unwrap();
        wait(&mut cx, |_| false);
        shot(&mut cx, "query-explain.png");
        dispatch(&mut cx, Box::new(navidog_core::actions::OpenHistoryLog));
        for _ in 0..3 {
            cx.advance_clock(std::time::Duration::from_millis(300));
            cx.run_until_parked();
        }
        shot(&mut cx, "history.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "history-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));

        // Share from the info pane's general page
        cx.update(|cx| {
            Workspace::global(cx).update(cx, |w, cx| {
                w.select(Selection::Object(actor_ref.clone()), cx)
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("info-share", cx);
        })
        .unwrap();
        shot(&mut cx, "share.png");
        cx.update_window(handle, |_, window, cx| window.close_all_dialogs(cx))
            .unwrap();

        // Manage Connections, searching highlights the matches
        dispatch(&mut cx, Box::new(navidog_core::actions::ManageConnections));
        cx.run_until_parked();
        let manage = cx
            .update(|cx| {
                cx.windows()
                    .into_iter()
                    .find(|w| w.window_id() != handle.window_id())
            })
            .unwrap();
        cx.update_window(manage, |_, window, cx| {
            window.render_frame(cx);
            window.click("manage-search", cx);
            window.input("sql", cx);
        })
        .unwrap();
        shot_of(&mut cx, manage, "manage.png");
        cx.update_window(manage, |_, window, cx| {
            let delta = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                // raw-px: scroll the connection table sideways to show the later columns
                gpui_kit::px(-240.),
                gpui_kit::Pixels::ZERO,
            ));
            window
                .within("manage-connections")
                .scroll(("manage-conn", 0usize), delta, cx);
        })
        .unwrap();
        shot_of(&mut cx, manage, "manage-scrolled.png");

        cx.update(navidog::preferences::open);
        cx.run_until_parked();
        let prefs = cx
            .update(|cx| {
                cx.windows().into_iter().find(|w| {
                    w.window_id() != handle.window_id() && w.window_id() != manage.window_id()
                })
            })
            .unwrap();
        shot_of(&mut cx, prefs, "preferences.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot_of(&mut cx, prefs, "preferences-dark.png");
        cx.update_window(prefs, |_, window, cx| {
            window.render_frame(cx);
            navidog::preferences::open_master_dialog(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        shot_of(&mut cx, prefs, "master-password.png");
        let _ = std::fs::remove_dir_all(home);
    }

    /// Waits for async loads (drivers run on the tokio runtime), at most 3 seconds.
    fn wait(cx: &mut HeadlessAppContext, done: impl Fn(&gpui_kit::App) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            cx.run_until_parked();
            if cx.update(|cx| done(cx)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Sample SQLite database: a few tables, a view and an index.
    fn sample_db(path: &std::path::Path) {
        let params = navidog_db::ConnectParams {
            file: Some(path.into()),
            create_new: true,
            ..Default::default()
        };
        // Enough tables to overflow the navigator, so the shot shows whether the tree fills the pane
        let mut logs: String = (1..=24)
            .map(|i| format!("CREATE TABLE log_{i:02}(id INTEGER PRIMARY KEY);"))
            .collect();
        // A wide table: its columns overflow the viewer, which must scroll horizontally
        let cols: Vec<String> = (0..30)
            .map(|i| format!("column_with_a_long_name_{i:02} TEXT"))
            .collect();
        logs.push_str(&format!(
            "CREATE TABLE wide(id INTEGER PRIMARY KEY, {});",
            cols.join(", ")
        ));
        logs.push_str("INSERT INTO wide(id) VALUES (1), (2);");
        // Names longer than the panes are truncated with an ellipsis, never wrapped
        logs.push_str("CREATE TABLE film_rental_statistics_by_store_and_category_monthly(id INTEGER PRIMARY KEY);");
        navidog_db::runtime::runtime().block_on(async {
            let mut s = navidog_db::connect(Arc::new(navidog_db::sqlite::SqliteDriver::default()), params).await.unwrap();
            s.relational()
                .unwrap()
                .execute(&format!(
                    "CREATE TABLE actor(id INTEGER PRIMARY KEY, first_name TEXT NOT NULL, last_name TEXT NOT NULL);\
                     CREATE TABLE film(id INTEGER PRIMARY KEY, title TEXT, release_year INTEGER, rating TEXT);\
                     CREATE TABLE film_actor(actor_id INTEGER, film_id INTEGER, PRIMARY KEY(actor_id, film_id));\
                     CREATE VIEW actor_names AS SELECT first_name || ' ' || last_name AS name FROM actor;\
                     CREATE INDEX idx_actor_last ON actor(last_name);\
                     INSERT INTO actor(first_name, last_name) VALUES ('PENELOPE','GUINESS'),('NICK','WAHLBERG');{logs}",
                ))
                .await
                .unwrap();
            s.close().await.unwrap();
        });
    }

    /// Seeds sample connections and a group, and selects one connection.
    fn seed(cx: &mut gpui_kit::App) {
        let store = ConnectionStore::global(cx);
        store.update(cx, |s, cx| {
            s.set_key(Key::random());
            let group = s.add_group("生产环境".into(), None, cx);
            let mut my = Connection::new("mysql");
            my.name = "本地 MySQL".into();
            my.color = Some("blue".into());
            let mut pg = Connection::new("postgres");
            pg.name = "pagila".into();
            pg.group = Some(group);
            let mut lite = Connection::new("sqlite");
            lite.name = "app.db".into();
            let file = navidog_core::paths::config_dir().join("app.db");
            sample_db(&file);
            lite.general.file = Some(file.to_string_lossy().into());
            for c in [my, pg, lite] {
                s.upsert(c, &[], cx).unwrap();
            }
        });
        let first = store.read(cx).connections()[0].id.clone();
        Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Connection(first), cx));
    }
}
