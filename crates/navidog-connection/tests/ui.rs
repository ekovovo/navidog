//! 连接管理的 headless UI 测试：通过真实的点击和键盘输入新建 SQLite 连接，并打开它。

use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, Window, WindowBounds, WindowOptions,
    div, size,
};
use navidog_connection::sessions::{ConnState, Sessions};
use navidog_connection::{NavigatorPanel, NewConnectionOf, OpenConnection};
use navidog_core::crypto::Key;
use navidog_core::store::ConnectionStore;

struct Host {
    focus: FocusHandle,
    navigator: Entity<NavigatorPanel>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        navidog_connection::bind_actions(
            div()
                .id("host")
                .track_focus(&self.focus)
                .size_full()
                .child(self.navigator.clone()),
        )
    }
}

#[gpui_kit::test]
fn creates_and_opens_sqlite_connection(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-ui-test-{}", std::process::id()));
    let db_file = home.join("app.db");
    // SAFETY: 本测试二进制只有这一个测试，设置时没有其他线程读取环境变量
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };

    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_connection::init(cx);
        // 不访问钥匙串
        ConnectionStore::global(cx).update(cx, |s, _| s.set_key(Key::random()));
    });
    let (handle, host) = cx.update(|cx| {
        // raw-px: 测试窗口尺寸
        let test_size = size(gpui_kit::px(1000.), gpui_kit::px(700.));
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

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.focus(&host.read(cx).focus.clone(), cx);
        window.dispatch_action(
            Box::new(NewConnectionOf {
                driver: "sqlite".into(),
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // 选“新建 SQLite 3”，填写文件路径和连接名
        // RadioGroup 以序号作为子项 ID
        window
            .within("sqlite-type")
            .click(gpui_kit::ElementId::Integer(1), cx);
        window.click("file-browse-input", cx);
        window.input(&db_file.to_string_lossy(), cx);
        window.click("conn-name", cx);
        window.press("cmd-a", cx);
        window.input("测试库", cx);
        window.click("ok", cx);
        window.render_frame(cx);
    })
    .unwrap();

    let id = cx.update(|cx| {
        let store = ConnectionStore::global(cx);
        let store = store.read(cx);
        assert_eq!(store.connections().len(), 1);
        let c = &store.connections()[0];
        assert_eq!((c.name.as_str(), c.driver.as_str()), ("测试库", "sqlite"));
        assert_eq!(
            c.general.file.as_deref(),
            Some(db_file.to_string_lossy().as_ref())
        );
        assert!(c.general.create_new);
        c.id.clone()
    });
    // 保存后表单已关闭，新连接成为当前选择
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("conn-name").is_none());
        window.focus(&host.read(cx).focus.clone(), cx);
        window.dispatch_action(Box::new(OpenConnection), cx);
    })
    .unwrap();

    // 驱动在 tokio 运行时中执行，等待结果回到前台
    cx.executor().allow_parking();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        let state = cx.update(|cx| Sessions::global(cx).read(cx).state(&id).clone());
        match state {
            ConnState::Open { databases, .. } => {
                assert_eq!(databases, ["main"]);
                break;
            }
            ConnState::Failed(e) => panic!("打开连接失败: {e}"),
            _ if Instant::now() > deadline => panic!("打开连接超时: {state:?}"),
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    assert!(db_file.exists(), "新建 SQLite 3 时创建文件");
    // 重新读取 connections.json：配置已持久化
    let on_disk = std::fs::read_to_string(home.join("connections.json")).unwrap();
    assert!(on_disk.contains("测试库"));
    let _ = std::fs::remove_dir_all(home);
}
