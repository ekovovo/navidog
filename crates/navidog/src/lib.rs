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
mod update;

use gpui_kit::component::{TitleBar, WindowExt as _};
use gpui_kit::{App, AppContext as _, Bounds, WindowBounds, WindowOptions};
use navidog_core::settings::Settings;
use navidog_ui::{NavidogAssets, tokens};
use rust_i18n::t;

use actions::*;
use main_window::MainWindow;
use state::AppState;

const REPO: &str = "https://github.com/xhofe/navidog";

/// The app log goes to the log folder from the preferences; "diagnostic log" records debug
/// messages too (F-PREF-010). A changed folder applies at the next start.
fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let settings = Settings::load();
    let dir = settings.log_dir();
    navidog_db::log::set_dir(dir.clone());
    let level = if settings.diagnostic_log {
        "debug"
    } else {
        "info"
    };
    let appender = tracing_appender::rolling::daily(dir, "navidog.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("NAVIDOG_LOG")
                .unwrap_or_else(|_| level.into()),
        )
        .with_writer(writer)
        .with_ansi(false)
        .init();
    guard
}

pub fn run() {
    let _log_guard = init_logging();
    update::arm_smoke_watchdog();
    gpui_kit::application()
        .with_assets(NavidogAssets)
        .run(|cx| {
            init(cx);
            cx.set_app_identity("ci.nn.navidog", "Navidog");
            set_dock_icon();
            menus::bind_keys(cx);
            menus::set_menus(cx);
            // Checked menu items follow the settings; the query and table menus follow the active tab
            cx.observe_global::<Settings>(menus::set_menus).detach();
            cx.observe_global::<navidog_ui::tabs::ActiveMenu>(menus::set_menus)
                .detach();
            register_app_actions(cx);

            let state = AppState::load();
            let (window, view) = open_main_window(state, cx);
            // The recovery dialog needs the window root, which exists only after open_window returns.
            let _ = window.update(cx, |_, window, cx| {
                view.update(cx, |v, cx| v.check_recovery(window, cx));
            });
            cx.on_app_quit(move |cx| {
                navidog_query::flush_on_quit(cx);
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
    navidog_query::init(cx);
    navidog_designer::init(cx);
    navidog_plugin::init(cx);
    register_commands(cx);
    cx.set_global(AppState::load());
}

/// The theme family and appearance from the settings, then the user's syntax colors.
pub fn apply_theme(window: Option<&mut gpui_kit::Window>, cx: &mut App) {
    use gpui_kit::component::ActiveTheme as _;
    use navidog_core::settings::ThemeFamily as Saved;
    use navidog_ui::theme::ThemeFamily as Active;
    let s = Settings::global(cx).clone();
    let family = match s.theme {
        Saved::Navidog => Active::Navidog,
        Saved::Default => Active::Default,
    };
    navidog_ui::theme::apply_family(family, cx);
    navidog_ui::theme::apply_appearance(s.appearance.is_dark(), window, cx);
    if family == Active::Default {
        navidog_ui::theme::pin_layout_font(cx);
    }
    let colors = if cx.theme().is_dark() {
        &s.syntax_colors.dark
    } else {
        &s.syntax_colors.light
    };
    navidog_ui::theme::apply_syntax_colors(colors, cx);
}

/// Shell commands shown on the main toolbar.
fn register_commands(cx: &mut App) {
    use navidog_core::registry::{CommandInfo, CommandRegistry, Source};
    let _ = cx.global_mut::<CommandRegistry>().0.register(
        "query.new",
        Source::Builtin,
        CommandInfo {
            title: "toolbar.new_query".into(),
            icon: Some("icons/square-terminal.svg".into()),
            shortcut: Some("cmd-y".into()),
            action: || Box::new(navidog_core::actions::NewQuery),
            enabled: None,
        },
    );
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
    gpui_kit::open_window(options, cx, move |window, cx| {
        apply_theme(Some(window), cx);
        update::arm_smoke_window(window);
        let view = cx.new(|cx| MainWindow::new(state, window, cx));
        if !update::is_smoke_test() {
            update::startup_check(window, cx);
        }
        if Settings::global(cx).auto_connect_on_startup {
            navidog_connection::auto_connect(window, cx);
        }
        view.update(cx, |v, cx| v.open_startup_tabs(window, cx));
        view
    })
    .expect("无法打开主窗口")
}

/// The key window, else the frontmost, else any window GPUI still has open.
fn front_window(cx: &App) -> Option<gpui_kit::AnyWindowHandle> {
    cx.active_window()
        .or_else(|| cx.window_stack().and_then(|stack| stack.into_iter().next()))
        .or_else(|| cx.windows().into_iter().next())
}

/// Menu actions are dispatched inside `window.update`. Updating that same
/// window again finds its slot empty and the `Result` was ignored, so the
/// click did nothing. Run after this update puts the window back.
fn on_front_window(cx: &mut App, f: impl FnOnce(&mut gpui_kit::Window, &mut App) + 'static) {
    let Some(window) = front_window(cx) else {
        tracing::warn!("menu action had no window");
        return;
    };
    cx.defer(move |cx| {
        if let Err(err) = window.update(cx, |_, window, cx| f(window, cx)) {
            tracing::warn!(%err, "menu action could not reach its window");
        }
    });
}

/// Wires the app-menu actions to their listeners. The menu bar itself is the platform part
/// (`menus::set_menus`); this side needs no platform UI, so headless tests use it too. Register
/// once per app: a second call adds a second listener for every action.
pub fn register_app_actions(cx: &mut App) {
    cx.on_action(|a: &navidog_core::actions::TableStructureChanged, cx| {
        navidog_navigator::catalog::Catalog::global(cx)
            .update(cx, |catalog, cx| catalog.refresh_scope(&a.scope, cx));
        navidog_db::outline::OutlineCache::global(cx)
            .update(cx, |cache, cx| cache.ensure(&a.scope, true, cx));
    });
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &ShowMainWindow, cx| cx.activate(true));
    cx.on_action(|_: &BringAllToFront, cx| cx.activate(true));
    cx.on_action(|_: &Preferences, cx| preferences::open(cx));
    cx.on_action(|_: &CheckForUpdates, cx| {
        on_front_window(cx, update::manual_check);
    });
    cx.on_action(|_: &OpenHelp, cx| cx.open_url(REPO));
    cx.on_action(|_: &ReportIssue, cx| cx.open_url(&format!("{REPO}/issues/new")));
    cx.on_action(|_: &About, cx| {
        on_front_window(cx, |window, cx| {
            window.open_alert_dialog(cx, |alert, _, _| {
                alert
                    .title("Navidog")
                    .description(format!(
                        "{}\n{}",
                        t!("about.version", version = update::current_label()),
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
    use std::ffi::{CStr, c_char, c_void};

    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        // Not variadic: on arm64 a variadic declaration uses another ABI, so
        // the extra arguments never arrive and AppKit throws.
        fn objc_msgSend();
    }

    const SVG: &[u8] = include_bytes!("../resources/navidog.svg");
    type Send0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type Send1 = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
    type SendBytes =
        unsafe extern "C" fn(*mut c_void, *mut c_void, *const u8, usize) -> *mut c_void;

    unsafe {
        let sel = |name: &CStr| sel_registerName(name.as_ptr());
        let send0: Send0 = std::mem::transmute(objc_msgSend as *const ());
        let send1: Send1 = std::mem::transmute(objc_msgSend as *const ());
        let send_bytes: SendBytes = std::mem::transmute(objc_msgSend as *const ());
        let data = send_bytes(
            objc_getClass(c"NSData".as_ptr()),
            sel(c"dataWithBytes:length:"),
            SVG.as_ptr(),
            SVG.len(),
        );
        let image = send1(
            send0(objc_getClass(c"NSImage".as_ptr()), sel(c"alloc")),
            sel(c"initWithData:"),
            data,
        );
        if image.is_null() {
            tracing::warn!("the dock icon SVG did not load");
            return;
        }
        let app = send0(
            objc_getClass(c"NSApplication".as_ptr()),
            sel(c"sharedApplication"),
        );
        send1(app, sel(c"setApplicationIconImage:"), image);
    }
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon() {}
