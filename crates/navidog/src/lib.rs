//! navidog app shell: initializes every layer, the menu bar and the main window (10-shell.md §1).

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

pub mod actions;
mod editors;
pub mod main_window;
mod menus;
mod opening;
mod popout;
pub mod preferences;
pub mod state;

use gpui_kit::component::{TitleBar, WindowExt as _};
use gpui_kit::{App, AppContext as _, Bounds, WindowBounds, WindowOptions};
use navidog_core::settings::Settings;
use navidog_ui::{NavidogAssets, tokens};
use rust_i18n::t;

use actions::*;
use main_window::MainWindow;
use state::AppState;

const REPO: &str = "https://github.com/xhofe/navidog";

fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let appender = tracing_appender::rolling::daily(navidog_core::paths::log_dir(), "navidog.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("NAVIDOG_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .with_writer(writer)
        .with_ansi(false)
        .init();
    guard
}

pub fn run() {
    let _log_guard = init_logging();
    gpui_kit::application()
        .with_assets(NavidogAssets)
        .run(|cx| {
            init(cx);
            cx.set_app_identity("ci.nn.navidog", "navidog");
            set_dock_icon();
            menus::bind_keys(cx);
            menus::set_menus(cx);
            register_app_actions(cx);

            let state = AppState::load();
            let (window, view) = open_main_window(state, cx);
            cx.on_app_quit(move |cx| {
                let view = view.clone();
                let _ = window.update(cx, |_, window, cx| {
                    view.update(cx, |v, cx| v.save_state(window, cx))
                });
                async {}
            })
            .detach();
            cx.activate(true);
        });
}

/// Initializes every layer (no platform UI such as the menu bar, so headless rendering uses it too).
pub fn init(cx: &mut App) {
    use gpui_kit::component as gpui_component;
    rust_i18n::extend!(gpui_component);
    gpui_kit::init(cx);
    navidog_core::init(cx);
    gpui_kit::component::set_locale(Settings::global(cx).language.locale());
    navidog_ui::theme::init(cx);
    navidog_ui::tabs::init(cx);
    navidog_db::init(cx);
    navidog_connection::init(cx);
    navidog_navigator::init(cx);
    navidog_grid::init(cx);
    navidog_plugin::init(cx);
    cx.set_global(AppState::load());
}

pub fn main_window_options(state: &AppState, cx: &mut App) -> WindowOptions {
    let bounds = state.window.map(|f| f.to_bounds()).unwrap_or_else(|| {
        WindowBounds::Windowed(Bounds::centered(None, tokens::default_window_size(), cx))
    });
    WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(tokens::min_window_size()),
        ..TitleBar::window_options()
    }
}

fn open_main_window(
    state: AppState,
    cx: &mut App,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<MainWindow>) {
    let options = main_window_options(&state, cx);
    let appearance = Settings::global(cx).appearance;
    gpui_kit::open_window(options, cx, move |window, cx| {
        navidog_ui::theme::apply_appearance(appearance.is_dark(), Some(window), cx);
        let view = cx.new(|cx| MainWindow::new(state, window, cx));
        if Settings::global(cx).auto_connect_on_startup {
            navidog_connection::auto_connect(window, cx);
        }
        view.update(cx, |v, cx| v.open_startup_tabs(window, cx));
        view
    })
    .expect("无法打开主窗口")
}

fn register_app_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &ShowMainWindow, cx| cx.activate(true));
    cx.on_action(|_: &BringAllToFront, cx| cx.activate(true));
    cx.on_action(|_: &Preferences, cx| preferences::open(cx));
    cx.on_action(|_: &CheckForUpdates, cx| cx.open_url(&format!("{REPO}/releases")));
    cx.on_action(|_: &OpenHelp, cx| cx.open_url(REPO));
    cx.on_action(|_: &ReportIssue, cx| cx.open_url(&format!("{REPO}/issues/new")));
    cx.on_action(|_: &About, cx| {
        let Some(window) = cx.active_window() else {
            return;
        };
        let _ = window.update(cx, |_, window, cx| {
            window.open_alert_dialog(cx, |alert, _, _| {
                alert
                    .title("navidog")
                    .description(format!(
                        "{}\n{}",
                        t!("about.version", version = env!("CARGO_PKG_VERSION")),
                        t!("about.description")
                    ))
                    .ok_text(t!("prefs.ok").to_string())
            });
        });
    });
}

/// Dock icon for `cargo run`. A bundled `.app` also gets this artwork from
/// `scripts/bundle-macos.sh`; gpui ignores `WindowOptions::icon` on macOS.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use std::ffi::{c_char, c_void, CStr};

    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend(receiver: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
    }

    const SVG: &[u8] = include_bytes!("../resources/navidog.svg");

    unsafe {
        let sel = |name: &CStr| sel_registerName(name.as_ptr());
        let data = objc_msgSend(
            objc_getClass(c"NSData".as_ptr()),
            sel(c"dataWithBytes:length:"),
            SVG.as_ptr(),
            SVG.len(),
        );
        let image = objc_msgSend(
            objc_msgSend(objc_getClass(c"NSImage".as_ptr()), sel(c"alloc")),
            sel(c"initWithData:"),
            data,
        );
        if image.is_null() {
            tracing::warn!("the dock icon SVG did not load");
            return;
        }
        let app = objc_msgSend(
            objc_getClass(c"NSApplication".as_ptr()),
            sel(c"sharedApplication"),
        );
        objc_msgSend(app, sel(c"setApplicationIconImage:"), image);
    }
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon() {}
