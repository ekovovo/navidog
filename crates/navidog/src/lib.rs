//! navidog app shell: initializes every layer, the menu bar and the main window (10-shell.md §1).

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

pub mod actions;
mod editors;
pub mod main_window;
mod menu_catalog;
mod menus;
mod opening;
mod popout;
pub mod preferences;
pub mod state;
mod update;

use gpui_kit::component::{TitleBar, WindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Global, WindowBounds, WindowOptions,
};
use navidog_app_state::settings::Settings;
use navidog_ui::{NavidogAssets, tokens};
use rust_i18n::t;

use actions::*;
use main_window::MainWindow;
use state::AppState;

const REPO: &str = "https://github.com/xhofe/navidog";

/// The app log goes to the log folder from the preferences; "diagnostic log" records debug
/// messages too (F-PREF-010). A changed folder applies at the next start.
fn init_logging() -> (Settings, tracing_appender::non_blocking::WorkerGuard) {
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
    (settings, guard)
}

pub fn run() {
    let (settings, _log_guard) = init_logging();
    update::arm_smoke_watchdog();
    let app = gpui_kit::application().with_assets(NavidogAssets);
    // `on_reopen` borrows the app; `run` takes it. The borrow ends on this line.
    app.on_reopen(present_main_window);
    app.run(move |cx| {
        init_with_settings(cx, settings);
        cx.set_app_identity("ci.nn.navidog", "Navidog");
        set_dock_icon();
        menus::bind_keys(cx);
        menus::set_menus(cx);
        // Checked menu items follow the settings; the query and table menus follow the active tab
        cx.observe_global::<Settings>(menus::set_menus).detach();
        cx.observe_global::<navidog_ui::tabs::ActiveMenu>(menus::set_menus)
            .detach();
        register_app_actions(cx);

        let state = cx.global::<AppState>().clone();
        let Some((window, view)) = open_main_window(state, true, cx) else {
            tracing::error!("cannot open the main window");
            cx.quit();
            return;
        };
        // The recovery dialog needs the window root, which exists only after open_window returns.
        let _ = window.update(cx, |_, window, cx| {
            view.update(cx, |v, cx| v.check_recovery(window, cx));
        });
        cx.on_app_quit(|cx| {
            navidog_query::flush_on_quit(cx);
            save_foreground_main(cx);
            async {}
        })
        .detach();
        cx.activate(true);
    });
}

/// Initializes every layer (no platform UI such as the menu bar, so headless rendering uses it too).
pub fn init(cx: &mut App) {
    init_with_settings(cx, Settings::load());
}

/// Like [`init`], with settings already read by the caller.
pub fn init_with_settings(cx: &mut App, settings: Settings) {
    use gpui_kit::component as gpui_component;
    rust_i18n::extend!(gpui_component);
    gpui_kit::init(cx);
    navidog_app_state::init_with_settings(cx, settings);
    gpui_kit::component::set_locale(Settings::global(cx).language.locale());
    navidog_ui::theme::init(cx);
    navidog_ui::tabs::init(cx);
    navidog_session::init(cx);
    navidog_connection::init(cx);
    navidog_navigator::init(cx);
    navidog_grid::init(cx);
    navidog_query::init(cx);
    navidog_designer::init(cx);
    menu_catalog::init(cx);
    cx.set_global(AppState::load());
}

/// The theme family and appearance from the settings, then the user's syntax colors.
pub fn apply_theme(window: Option<&mut gpui_kit::Window>, cx: &mut App) {
    use gpui_kit::component::ActiveTheme as _;
    use navidog_app_state::settings::ThemeFamily as Saved;
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

/// The main window last opened. Quit saves this one, including after a Dock reopen
/// replaced the window created at startup.
struct ForegroundMain {
    window: AnyWindowHandle,
    view: Entity<MainWindow>,
}

impl Global for ForegroundMain {}

fn open_main_window(
    state: AppState,
    startup: bool,
    cx: &mut App,
) -> Option<(AnyWindowHandle, Entity<MainWindow>)> {
    let options = main_window_options(&state, cx);
    let (window, view) = gpui_kit::open_window(options, cx, move |window, cx| {
        apply_theme(Some(window), cx);
        update::arm_smoke_window(window);
        let view = cx.new(|cx| MainWindow::new(state, window, cx));
        if startup {
            if !update::is_smoke_test() {
                update::startup_check(window, cx);
            }
            if Settings::global(cx).auto_connect_on_startup {
                navidog_connection::auto_connect(window, cx);
            }
            view.update(cx, |v, cx| v.open_startup_tabs(window, cx));
        }
        view
    })
    .ok()?;
    cx.set_global(ForegroundMain {
        window,
        view: view.clone(),
    });
    Some((window, view))
}

fn save_foreground_main(cx: &mut App) {
    let Some(main) = cx.try_global::<ForegroundMain>() else {
        return;
    };
    let window = main.window;
    let view = main.view.clone();
    let _ = window.update(cx, |_, window, cx| {
        view.update(cx, |v, cx| v.save_state(window, cx))
    });
}

/// Brings the main window forward. A window that is still open, including a
/// minimized one, is activated. With none left, opens one from the saved state
/// and skips startup work (update check, auto-connect, startup tabs, recovery).
/// The Dock icon and Window > Show Main Window both call this.
pub fn present_main_window(cx: &mut App) {
    // A closed main window can still have a tracked handle, so ask the open
    // windows whether it is still there.
    if let Some(window) = cx.try_global::<ForegroundMain>().map(|main| main.window)
        && cx.windows().contains(&window)
    {
        // Menu commands run inside the key window's update, where that same
        // window's slot is empty and updating it again fails. Activate after
        // the slot is back instead of opening a second main window.
        if window
            .update(cx, |_, window, _| window.activate_window())
            .is_err()
        {
            cx.defer(move |cx| {
                if let Err(err) = window.update(cx, |_, window, _| window.activate_window()) {
                    tracing::warn!(%err, "cannot activate the main window");
                }
            });
        }
        cx.activate(true);
        return;
    }
    let state = cx.global::<AppState>().clone();
    if open_main_window(state, false, cx).is_none() {
        tracing::error!("cannot open the main window");
        return;
    }
    cx.activate(true);
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
    cx.on_action(
        |a: &navidog_app_state::actions::TableStructureChanged, cx| {
            navidog_navigator::catalog::Catalog::global(cx)
                .update(cx, |catalog, cx| catalog.refresh_scope(&a.scope, cx));
            navidog_session::outline::OutlineCache::global(cx)
                .update(cx, |cache, cx| cache.ensure(&a.scope, true, cx));
        },
    );
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &ShowMainWindow, cx| present_main_window(cx));
    cx.on_action(|_: &BringAllToFront, cx| cx.activate(true));
    cx.on_action(|_: &Preferences, cx| preferences::open(cx));
    cx.on_action(|_: &CheckForUpdates, cx| {
        on_front_window(cx, update::manual_check);
    });
    cx.on_action(|_: &OpenHelp, cx| cx.open_url(REPO));
    cx.on_action(|_: &ReportIssue, cx| cx.open_url(&format!("{REPO}/issues/new")));
    cx.on_action(|_: &navidog_app_state::actions::NewConnection, cx| {
        cx.stop_propagation();
        on_front_window(cx, |window, cx| {
            navidog_connection::open_type_picker(window, cx)
        });
    });
    cx.on_action(|a: &navidog_app_state::actions::NewConnectionOf, cx| {
        cx.stop_propagation();
        let driver = a.driver.clone();
        on_front_window(cx, move |window, cx| {
            navidog_connection::open_new(driver, window, cx);
        });
    });
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

/// Dock icon for `cargo run`. A bundled `.app` gets `navidog.icns` from
/// `scripts/bundle-macos.sh` or `cargo bundle`. gpui ignores `WindowOptions::icon`
/// on macOS, so the running Dock tile still comes from this image.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    const SVG: &[u8] = include_bytes!("../resources/navidog.svg");
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("the dock icon must be set on the main thread");
        return;
    };
    let data =
        unsafe { NSData::initWithBytes_length(NSData::alloc(), SVG.as_ptr().cast(), SVG.len()) };
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        tracing::warn!("the dock icon SVG did not load");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon() {}
