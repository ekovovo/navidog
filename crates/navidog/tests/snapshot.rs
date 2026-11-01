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
    use navidog_app_state::actions::{
        NewConnection, NewConnectionOf, OpenObject, ReopenConnectionId,
    };
    use navidog_app_state::settings::Settings;
    use navidog_app_state::store::ConnectionStore;
    use navidog_app_state::workspace::{Selection, Workspace};
    use navidog_model::connection::Connection;
    use navidog_model::crypto::Key;
    use navidog_model::object::{ObjectKind, ObjectRef, Scope};
    use navidog_session::sessions::Sessions;
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
            cx.set_reduce_motion(true);
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
        // One frame, before async loads park. `shot` waits long enough for SQLite to finish.
        let shot_now = |cx: &mut HeadlessAppContext, name: &str| {
            cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                .unwrap();
            let image = cx
                .capture_screenshot(handle)
                .expect("needs the Metal renderer");
            image.save(out.join(name)).unwrap();
            println!("saved {}", out.join(name).display());
        };
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

        // Failed persistence keeps the editable connection form open with its entered values.
        let original_connections =
            cx.update(|cx| ConnectionStore::global(cx).read(cx).file().clone());
        let original_bytes = std::fs::read(home.join("connections.json")).unwrap();
        // `write_atomic` uses a unique temp name, so a `connections.tmp` directory no longer
        // blocks it. Removing write permission on the home directory does.
        let writable = std::fs::metadata(&home).unwrap().permissions();
        let mut locked = writable.clone();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            locked.set_mode(locked.mode() & !0o222);
        }
        #[cfg(not(unix))]
        locked.set_readonly(true);
        std::fs::set_permissions(&home, locked).unwrap();
        dispatch(
            &mut cx,
            Box::new(NewConnectionOf {
                driver: "mysql".into(),
            }),
        );
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("conn-name", cx);
            window.press("secondary-a", cx);
            window.input("保存失败示例", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("ok", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("conn-name").is_some(),
                "failed save retains the form"
            );
            assert!(
                ConnectionStore::global(cx).read(cx).file() == &original_connections,
                "failed save retains the existing connections"
            );
        })
        .unwrap();
        assert!(
            std::fs::read(home.join("connections.json")).unwrap() == original_bytes,
            "failed save leaves the original configuration on disk"
        );
        for (mode, name) in [
            (ThemeMode::Light, "connection-save-failed-light.png"),
            (ThemeMode::Dark, "connection-save-failed-dark.png"),
        ] {
            cx.update(|cx| Theme::change(mode, None, cx));
            shot(&mut cx, name);
        }
        std::fs::set_permissions(&home, writable).unwrap();
        cx.update_window(handle, |_, window, cx| window.close_all_dialogs(cx))
            .unwrap();
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));

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
        shot_now(&mut cx, "objects-loading.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot_now(&mut cx, "objects-loading-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
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
            window.click("toggle-filter", cx);
        })
        .unwrap();
        shot(&mut cx, "viewer-filter.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "viewer-filter-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("toggle-filter", cx);
        })
        .unwrap();
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

        dispatch(&mut cx, Box::new(ReopenConnectionId::new(lite.clone())));
        cx.run_until_parked();
        shot(&mut cx, "session-reopen-light.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "session-reopen-dark.png");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.has_active_dialog(cx));
            window.click("cancel", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(cx.update(|cx| Sessions::global(cx).read(cx).state(&lite).is_open()));
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
        dispatch(&mut cx, Box::new(navidog_app_state::actions::NewQuery));
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
        dispatch(
            &mut cx,
            Box::new(navidog_app_state::actions::OpenHistoryLog),
        );
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

        // M4: open an existing table design and preview a pending field addition.
        dispatch(
            &mut cx,
            Box::new(navidog_app_state::actions::DesignTable {
                object: Some(actor_ref.clone()),
            }),
        );
        shot_now(&mut cx, "designer-loading.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot_now(&mut cx, "designer-loading-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        wait(&mut cx, |_| false);
        shot(&mut cx, "designer.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "designer-dark.png");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("designer-add", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("designer-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        shot(&mut cx, "designer-preview-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        shot(&mut cx, "designer-preview.png");

        // Table designer pages and a narrow viewport retain their structured controls.
        for (page, name) in [
            ("fields", "designer-grid"),
            ("indexes", "designer-indexes"),
            ("foreign_keys", "designer-foreign-keys"),
            ("checks", "designer-checks"),
            ("options", "designer-options"),
        ] {
            cx.update_window(handle, |_, w, cx| {
                w.click(
                    gpui_kit::SharedString::from(format!("designer-page-{page}")),
                    cx,
                )
            })
            .unwrap();
            cx.run_until_parked();
            if page != "fields" && page != "options" {
                cx.update_window(handle, |_, w, cx| w.click("designer-add", cx))
                    .unwrap();
                cx.run_until_parked();
            }
            if page == "foreign_keys" {
                cx.update_window(handle, |_, w, cx| {
                    w.click("designer-entry-reference_table", cx);
                    w.press("secondary-a", cx);
                    w.input("actor", cx);
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(handle, |_, w, cx| {
                    w.click("designer-entry-reference_fields", cx);
                    w.input("\"id\"", cx);
                })
                .unwrap();
                wait(&mut cx, |_| false);
            }
            for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                cx.update(|cx| Theme::change(mode, None, cx));
                shot(&mut cx, &format!("{name}-{suffix}.png"));
            }
        }
        cx.update_window(handle, |_, w, cx| w.click("designer-page-fields", cx))
            .unwrap();
        cx.run_until_parked();
        dispatch(&mut cx, Box::new(navidog::actions::ToggleInfoPane));
        cx.run_until_parked();
        shot(&mut cx, "designer-wide-dark.png");
        // TestPlatform::resize changes only the capture dimensions, not the layout callback.
        // Open a genuinely narrow designer viewport instead of cropping the full-size frame.
        // raw-px: headless window rem size resolved from the shared font token.
        let snapshot_rem = gpui_kit::px(navidog_ui::tokens::font::SIZE_BODY.value());
        let small = cx
            .update(|cx| {
                gpui_kit::open_window(
                    gpui_kit::WindowOptions {
                        show: false,
                        focus: false,
                        window_bounds: Some(gpui_kit::WindowBounds::Windowed(
                            gpui_kit::Bounds::new(
                                gpui_kit::point(gpui_kit::Pixels::ZERO, gpui_kit::Pixels::ZERO),
                                gpui_kit::size(
                                    navidog_ui::tokens::window::MIN_WIDTH.to_pixels(snapshot_rem),
                                    navidog_ui::tokens::window::MIN_HEIGHT.to_pixels(snapshot_rem),
                                ),
                            ),
                        )),
                        ..Default::default()
                    },
                    cx,
                    |w, cx| navidog_designer::TableDesigner::open(actor_ref.clone(), w, cx),
                )
            })
            .unwrap();
        wait(&mut cx, |_| false);
        shot_of(&mut cx, small.0, "designer-minimum-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        shot_of(&mut cx, small.0, "designer-minimum-light.png");
        cx.update_window(small.0, |_, w, _| w.remove_window())
            .unwrap();
        dispatch(
            &mut cx,
            Box::new(navidog_app_state::actions::OpenDefinition {
                scope: actor_ref.scope.clone(),
                kind: "view".into(),
                name: None,
            }),
        );
        cx.run_until_parked();
        shot(&mut cx, "object-designer-light.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "object-designer-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        dispatch(
            &mut cx,
            Box::new(navidog_app_state::actions::OpenTool {
                scope: Some(actor_ref.scope.clone()),
                tool: "users".into(),
                target: None,
            }),
        );
        cx.run_until_parked();
        shot(&mut cx, "users-light.png");
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        shot(&mut cx, "users-dark.png");
        cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
        dispatch(&mut cx, Box::new(navidog::actions::ToggleInfoPane));
        // An independent SQL session changes a table that is already open in the viewer.
        let refresh_file = home.join("app.db");
        execute_sample_sql(
            &refresh_file,
            "CREATE TABLE refresh_example(id INTEGER PRIMARY KEY, name TEXT); \
             INSERT INTO refresh_example VALUES (1, '等待查询执行')",
        );
        let refresh_settings = cx.update(|cx| {
            let settings = Settings::global(cx);
            let saved = (settings.sync_record, settings.auto_begin_transaction);
            Settings::update(cx, |settings| {
                settings.sync_record = false;
                settings.auto_begin_transaction = false;
            });
            saved
        });
        dispatch(
            &mut cx,
            Box::new(navidog_app_state::actions::ToggleFocusMode),
        );
        dispatch(
            &mut cx,
            Box::new(OpenObject {
                object: ObjectRef {
                    scope: actor_ref.scope.clone(),
                    kind: ObjectKind::Table,
                    name: "refresh_example".into(),
                },
            }),
        );
        wait_for_table(&mut cx, handle, "等待查询执行");
        execute_sample_sql(
            &refresh_file,
            "ALTER TABLE refresh_example ADD COLUMN source TEXT; \
             UPDATE refresh_example SET name = 'SQL 已更新', source = '查询面板'; \
             INSERT INTO refresh_example VALUES (2, '新增记录', '查询面板')",
        );
        cx.update(|cx| {
            Workspace::global(cx).update(cx, |workspace, cx| {
                workspace.database_changed(lite.clone(), cx)
            });
        });
        wait_for_table(&mut cx, handle, "SQL 已更新\t查询面板");
        for (mode, name) in [
            (ThemeMode::Light, "viewer-auto-refresh-light.png"),
            (ThemeMode::Dark, "viewer-auto-refresh-dark.png"),
        ] {
            cx.update(|cx| Theme::change(mode, None, cx));
            shot(&mut cx, name);
        }

        // Enter a real cell edit, then keep that draft when another SQL session updates the row.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let rem = window.rem_size();
            let font = navidog_ui::tokens::font::SIZE_BODY.to_pixels(rem).as_f32();
            let x = navidog_ui::tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem)
                + navidog_grid::grid::auto_width("id", ["1", "2"], font)
                + navidog_grid::grid::auto_width("name", ["SQL 已更新", "新增记录"], font) / 2.;
            window.click_at(
                gpui_kit::ElementId::NamedInteger("row".into(), 0),
                gpui_kit::point(x, rem / 2.),
                cx,
            );
            window.press("enter", cx);
            window.render_frame(cx);
            window.press("secondary-a", cx);
            window.input("保留中的本地编辑", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        wait_for_table(&mut cx, handle, "保留中的本地编辑");
        execute_sample_sql(
            &refresh_file,
            "UPDATE refresh_example SET name = '数据库的后续值' WHERE id = 1",
        );
        cx.update(|cx| {
            Workspace::global(cx).update(cx, |workspace, cx| {
                workspace.database_changed(lite.clone(), cx)
            });
        });
        cx.run_until_parked();
        wait_for_table(&mut cx, handle, "保留中的本地编辑");
        assert_eq!(
            execute_sample_sql(
                &refresh_file,
                "SELECT name FROM refresh_example WHERE id = 1"
            ),
            [vec![Some("数据库的后续值".into())]],
            "the notification retains local input without committing it"
        );
        for (mode, name) in [
            (ThemeMode::Light, "viewer-auto-refresh-pending-light.png"),
            (ThemeMode::Dark, "viewer-auto-refresh-pending-dark.png"),
        ] {
            cx.update(|cx| Theme::change(mode, None, cx));
            shot(&mut cx, name);
        }
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("row-discard", cx);
        })
        .unwrap();
        dispatch(
            &mut cx,
            Box::new(navidog_app_state::actions::ToggleFocusMode),
        );
        cx.update(|cx| {
            Theme::change(ThemeMode::Light, None, cx);
            Settings::update(cx, |settings| {
                settings.sync_record = refresh_settings.0;
                settings.auto_begin_transaction = refresh_settings.1;
            });
        });

        // Manage Connections, searching highlights the matches
        dispatch(
            &mut cx,
            Box::new(navidog_app_state::actions::ManageConnections),
        );
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
        // Optional real-server snapshots also exercise the dialect option forms.
        if std::env::var_os("NAVIDOG_LIVE").is_some() {
            for (driver, port, database, user, key, value) in [
                ("mysql", 13306, "sakila", "root", "engine", "InnoDB"),
                ("postgres", 15432, "pagila", "postgres", "fillfactor", "80"),
            ] {
                let id = cx.update(|cx| {
                    ConnectionStore::global(cx).update(cx, |store, cx| {
                        let mut c = Connection::new(driver);
                        c.name = format!("{driver} options");
                        c.general.host = "127.0.0.1".into();
                        c.general.port = Some(port);
                        c.general.database = Some(database.into());
                        c.general.user = user.into();
                        let id = c.id.clone();
                        store.upsert(c, &[], cx).unwrap();
                        id
                    })
                });
                cx.update(|cx| {
                    Sessions::global(cx).update(cx, |s, cx| {
                        s.open(
                            id.clone(),
                            navidog_db::Secrets {
                                password: Some("navidog".into()),
                                ..Default::default()
                            },
                            cx,
                        )
                    })
                });
                wait(&mut cx, |cx| {
                    Sessions::global(cx).read(cx).state(&id).is_open()
                });
                assert!(cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open()));
                let mut panel = None;
                let options_window = cx
                    .update(|cx| {
                        gpui_kit::open_window(
                            navidog::main_window_options(&AppState::default(), cx),
                            cx,
                            |w, cx| {
                                let p = navidog_designer::TableDesigner::new(
                                    Scope::new(id.clone(), database, Some("public".into())),
                                    w,
                                    cx,
                                );
                                panel = Some(p.clone());
                                p
                            },
                        )
                    })
                    .unwrap();
                cx.update_window(options_window.0, |_, w, cx| {
                    w.render_frame(cx);
                    w.click("designer-table-name", cx);
                    w.input("new_table", cx);
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(options_window.0, |_, w, cx| {
                    w.click("designer-page-options", cx)
                })
                .unwrap();
                cx.run_until_parked();
                let input = gpui_kit::SharedString::from(format!("designer-option-{key}"));
                cx.update_window(options_window.0, |_, w, cx| {
                    w.click(input.clone(), cx);
                    w.input(value, cx);
                })
                .unwrap();
                cx.run_until_parked();
                if driver == "postgres" {
                    cx.update_window(options_window.0, |_, w, cx| {
                        w.click(input.clone(), cx);
                        w.press("secondary-a", cx);
                        w.input("101", cx);
                    })
                    .unwrap();
                    cx.run_until_parked();
                    cx.update_window(options_window.0, |_, w, cx| w.click("designer-save", cx))
                        .unwrap();
                    cx.run_until_parked();
                    cx.update_window(options_window.0, |_, w, cx| {
                        w.render_frame(cx);
                        assert!(w.try_find("designer-execute").is_none());
                        w.click(input.clone(), cx);
                        w.press("secondary-a", cx);
                        w.input("80", cx);
                    })
                    .unwrap();
                    cx.run_until_parked();
                    cx.update_window(options_window.0, |_, w, cx| {
                        w.click("designer-option-unlogged", cx)
                    })
                    .unwrap();
                    cx.run_until_parked();
                }
                for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                    cx.update(|cx| Theme::change(mode, None, cx));
                    shot_of(
                        &mut cx,
                        options_window.0,
                        &format!("designer-options-{driver}-{suffix}.png"),
                    );
                }
                if driver == "mysql" {
                    cx.update_window(options_window.0, |_, w, cx| {
                        w.scroll(
                            input.clone(),
                            gpui_kit::ScrollDelta::Lines(gpui_kit::point(0., -16.)),
                            cx,
                        )
                    })
                    .unwrap();
                    cx.run_until_parked();
                    cx.update_window(options_window.0, |_, w, cx| {
                        w.click("designer-option-table_comment", cx);
                        w.input("Table options preview", cx);
                    })
                    .unwrap();
                    cx.run_until_parked();
                    for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                        cx.update(|cx| Theme::change(mode, None, cx));
                        shot_of(
                            &mut cx,
                            options_window.0,
                            &format!("designer-options-mysql-bottom-{suffix}.png"),
                        );
                    }
                }
                cx.update_window(options_window.0, |_, w, cx| {
                    w.click("designer-page-fields", cx)
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(options_window.0, |_, w, cx| {
                    if driver == "postgres" {
                        w.click("designer-field-identity", cx);
                        w.input("ALWAYS", cx);
                    } else {
                        w.click("designer-zerofill", cx);
                    }
                })
                .unwrap();
                cx.run_until_parked();
                for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                    cx.update(|cx| Theme::change(mode, None, cx));
                    shot_of(
                        &mut cx,
                        options_window.0,
                        &format!("designer-attributes-{driver}-{suffix}.png"),
                    );
                }
                cx.update(|cx| {
                    use gpui_kit::component::dock::{BasePanel as _, PanelInfo};
                    let PanelInfo::Panel(value) = panel.as_ref().unwrap().read(cx).dump(cx).info
                    else {
                        panic!("table draft");
                    };
                    if driver == "mysql" {
                        assert!(
                            value["draft"]["options"]
                                .as_str()
                                .unwrap()
                                .contains("Table options preview")
                        );
                        assert!(
                            value["draft"]["columns"][0]["definition"]
                                .as_str()
                                .unwrap()
                                .contains("ZEROFILL")
                        );
                    } else {
                        assert_eq!(value["draft"]["unlogged"], true);
                        assert!(
                            value["draft"]["columns"][0]["definition"]
                                .as_str()
                                .unwrap()
                                .contains("GENERATED ALWAYS AS IDENTITY")
                        );
                    }
                });
                cx.update_window(options_window.0, |_, w, cx| w.click("designer-save", cx))
                    .unwrap();
                cx.run_until_parked();
                cx.update_window(options_window.0, |_, w, cx| {
                    w.render_frame(cx);
                    assert!(w.try_find("designer-execute").is_some());
                })
                .unwrap();
                cx.update_window(options_window.0, |_, w, _| w.remove_window())
                    .unwrap();
                if driver == "postgres" {
                    use gpui_kit::component::dock::{BasePanel as _, PanelInfo};
                    let name = format!("navidog_comment_snapshot_{}", std::process::id());
                    let table = format!("public.\"{name}\"");
                    let sql = format!(
                        "CREATE TABLE {table}(id INTEGER PRIMARY KEY); COMMENT ON TABLE {table} IS '原表注释'; COMMENT ON COLUMN {table}.id IS '原字段注释'"
                    );
                    let job = cx
                        .update(|cx| {
                            Sessions::global(cx)
                                .read(cx)
                                .run_rel(&id, Some(database), move |r| {
                                    Box::pin(async move { r.execute(&sql).await })
                                })
                        })
                        .unwrap();
                    navidog_db::runtime::runtime()
                        .block_on(job)
                        .unwrap()
                        .unwrap();
                    let mut comments = None;
                    let comments_window = cx
                        .update(|cx| {
                            gpui_kit::open_window(
                                navidog::main_window_options(&AppState::default(), cx),
                                cx,
                                |w, cx| {
                                    let p = navidog_designer::TableDesigner::open(
                                        ObjectRef {
                                            scope: Scope::new(
                                                id.clone(),
                                                database,
                                                Some("public".into()),
                                            ),
                                            kind: ObjectKind::Table,
                                            name: name.clone(),
                                        },
                                        w,
                                        cx,
                                    );
                                    comments = Some(p.clone());
                                    p
                                },
                            )
                        })
                        .unwrap();
                    let comments = comments.unwrap();
                    wait(&mut cx, |cx| match comments.read(cx).dump(cx).info {
                        PanelInfo::Panel(value) => value["baseline"].is_object(),
                        _ => false,
                    });
                    cx.update(|cx| {
                        let PanelInfo::Panel(value) = comments.read(cx).dump(cx).info else {
                            panic!("comments draft")
                        };
                        assert_eq!(value["draft"]["comment"], "原表注释");
                        assert_eq!(value["draft"]["columns"][0]["comment"], "原字段注释");
                    });
                    cx.update_window(comments_window.0, |_, w, cx| {
                        w.render_frame(cx);
                        w.render_frame(cx);
                        let row = w.find(("row", 0usize)).bounds();
                        let col = w.find(("col-header", 7usize)).bounds();
                        w.click_at(
                            ("row", 0usize),
                            gpui_kit::point(
                                col.left() - row.left() + col.size.width / 4.,
                                row.size.height / 2.,
                            ),
                            cx,
                        );
                    })
                    .unwrap();
                    cx.run_until_parked();
                    cx.update_window(comments_window.0, |_, w, cx| {
                        w.click("designer-field-comment", cx);
                        w.press("secondary-a", cx);
                        w.input("客户编号 O'Brien", cx);
                    })
                    .unwrap();
                    cx.run_until_parked();
                    for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                        cx.update(|cx| Theme::change(mode, None, cx));
                        shot_of(
                            &mut cx,
                            comments_window.0,
                            &format!("designer-comments-postgres-fields-{suffix}.png"),
                        );
                    }
                    cx.update_window(comments_window.0, |_, w, cx| {
                        w.click("designer-page-options", cx)
                    })
                    .unwrap();
                    cx.run_until_parked();
                    cx.update_window(comments_window.0, |_, w, cx| {
                        w.click("designer-option-table_comment", cx);
                        w.press("secondary-a", cx);
                        w.input("客户资料表 O'Brien", cx);
                    })
                    .unwrap();
                    cx.run_until_parked();
                    for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                        cx.update(|cx| Theme::change(mode, None, cx));
                        shot_of(
                            &mut cx,
                            comments_window.0,
                            &format!("designer-comments-postgres-table-{suffix}.png"),
                        );
                    }
                    cx.update_window(comments_window.0, |_, w, cx| w.click("designer-save", cx))
                        .unwrap();
                    cx.run_until_parked();
                    for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                        cx.update(|cx| Theme::change(mode, None, cx));
                        shot_of(
                            &mut cx,
                            comments_window.0,
                            &format!("designer-comments-postgres-preview-{suffix}.png"),
                        );
                    }
                    cx.update_window(comments_window.0, |_, w, _| w.remove_window())
                        .unwrap();
                    let sql = format!("DROP TABLE {table}");
                    let job = cx
                        .update(|cx| {
                            Sessions::global(cx)
                                .read(cx)
                                .run_rel(&id, Some(database), move |r| {
                                    Box::pin(async move { r.execute(&sql).await })
                                })
                        })
                        .unwrap();
                    navidog_db::runtime::runtime()
                        .block_on(job)
                        .unwrap()
                        .unwrap();
                }
                cx.update(|cx| Sessions::global(cx).update(cx, |s, cx| s.close(&id, cx)));
            }
        }
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

    /// Reads the actual grid selection through its normal Copy action until a load has finished.
    fn wait_for_table(
        cx: &mut HeadlessAppContext,
        handle: gpui_kit::AnyWindowHandle,
        expected: &str,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            let found = cx
                .update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    let row = gpui_kit::ElementId::NamedInteger("row".into(), 0);
                    if window.try_find(row.clone()).is_none() {
                        return false;
                    }
                    window.click_at(
                        row,
                        gpui_kit::point(
                            navidog_ui::tokens::grid::ROW_NUMBER_WIDTH.to_pixels(window.rem_size())
                                / 2.,
                            window.rem_size() / 2.,
                        ),
                        cx,
                    );
                    true
                })
                .unwrap();
            if found {
                cx.run_until_parked();
                let value = cx
                    .update_window(handle, |_, window, cx| {
                        window.render_frame(cx);
                        window.dispatch_action(Box::new(navidog_grid::grid::SelectAll), cx);
                        window.dispatch_action(Box::new(navidog_grid::grid::Copy), cx);
                        cx.read_from_clipboard().and_then(|item| item.text())
                    })
                    .unwrap();
                if value.is_some_and(|value| value.contains(expected)) {
                    return;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the visible table never contained {expected:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn execute_sample_sql(path: &std::path::Path, sql: &str) -> Vec<Vec<Option<String>>> {
        navidog_db::runtime::runtime().block_on(async {
            let mut session = navidog_db::connect(
                Arc::new(navidog_sqlite::SqliteDriver::default()),
                navidog_db::ConnectParams {
                    file: Some(path.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let sets = session.relational().unwrap().execute(sql).await.unwrap();
            session.close().await.unwrap();
            sets.into_iter()
                .last()
                .map(|set| set.rows)
                .unwrap_or_default()
        })
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
            let mut s = navidog_db::connect(Arc::new(navidog_sqlite::SqliteDriver::default()), params).await.unwrap();
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
            let group = s.add_group("生产环境".into(), None, cx).unwrap();
            let mut my = Connection::new("mysql");
            my.name = "本地 MySQL".into();
            my.color = Some("blue".into());
            let mut pg = Connection::new("postgres");
            pg.name = "pagila".into();
            pg.group = Some(group);
            let mut lite = Connection::new("sqlite");
            lite.name = "app.db".into();
            let file = navidog_model::paths::config_dir().join("app.db");
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
