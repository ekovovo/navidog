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
    use navidog_connection::{NewConnection, NewConnectionOf};
    use navidog_core::connection::Connection;
    use navidog_core::crypto::Key;
    use navidog_core::store::ConnectionStore;
    use navidog_core::workspace::{Selection, Workspace};
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
        let _ = std::fs::remove_dir_all(home);
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
            for c in [my, pg, lite] {
                s.upsert(c, &[], cx).unwrap();
            }
        });
        let first = store.read(cx).connections()[0].id.clone();
        Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Connection(first), cx));
    }
}
