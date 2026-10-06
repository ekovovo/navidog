//! 用 Metal headless 渲染器把主窗口画成 PNG，供并排目测（不做像素断言）。
//! 运行：`cargo test -p navidog --test snapshot`，输出在 `target/snapshots/`。
//! AppKit 要求主线程，所以 `harness = false`。

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
    use gpui_kit::{AppContext as _, Focusable as _, HeadlessAppContext};
    use navidog::main_window::MainWindow;
    use navidog::state::AppState;
    use navidog_core::actions::{NewConnection, NewConnectionOf, OpenObject};
    use navidog_core::connection::Connection;
    use navidog_core::crypto::Key;
    use navidog_core::object::{ObjectKind, ObjectRef, Scope};
    use navidog_core::store::ConnectionStore;
    use navidog_core::workspace::{Selection, Workspace};
    use navidog_db::sessions::Sessions;
    use navidog_ui::NavidogAssets;

    pub fn run() {
        let home = std::env::temp_dir().join(format!("navidog-snapshot-{}", std::process::id()));
        // SAFETY: 单线程启动阶段，尚未创建任何读取环境变量的线程
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

        let shot = |cx: &mut HeadlessAppContext, name: &str| {
            // 图标等资源异步加载，先让任务跑完再渲染
            for _ in 0..3 {
                cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                    .unwrap();
                cx.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let image = cx.capture_screenshot(handle).expect("需要 Metal 渲染器");
            image.save(out.join(name)).unwrap();
            println!("已保存 {}", out.join(name).display());
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

        // M2：打开 SQLite 示例库，展开到对象，显示对象列表与 DDL 选项卡
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
        dispatch(&mut cx, Box::new(OpenObject { object: actor }));
        wait(&mut cx, |_| false);
        shot(&mut cx, "viewer.png");
        let _ = std::fs::remove_dir_all(home);
    }

    /// 等待异步加载（驱动在 tokio 运行时中执行）；条件不满足时最多等 3 秒。
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

    /// SQLite 示例库：几张表、一个视图、一个索引。
    fn sample_db(path: &std::path::Path) {
        let params = navidog_db::ConnectParams {
            file: Some(path.into()),
            create_new: true,
            ..Default::default()
        };
        navidog_db::runtime::runtime().block_on(async {
            let mut s = navidog_db::connect(Arc::new(navidog_db::sqlite::SqliteDriver::default()), params).await.unwrap();
            s.relational()
                .unwrap()
                .execute(
                    "CREATE TABLE actor(id INTEGER PRIMARY KEY, first_name TEXT NOT NULL, last_name TEXT NOT NULL);\
                     CREATE TABLE film(id INTEGER PRIMARY KEY, title TEXT, release_year INTEGER, rating TEXT);\
                     CREATE TABLE film_actor(actor_id INTEGER, film_id INTEGER, PRIMARY KEY(actor_id, film_id));\
                     CREATE VIEW actor_names AS SELECT first_name || ' ' || last_name AS name FROM actor;\
                     CREATE INDEX idx_actor_last ON actor(last_name);\
                     INSERT INTO actor(first_name, last_name) VALUES ('PENELOPE','GUINESS'),('NICK','WAHLBERG');",
                )
                .await
                .unwrap();
            s.close().await.unwrap();
        });
    }

    /// 准备几条示例连接与分组，选中其中一条。
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
