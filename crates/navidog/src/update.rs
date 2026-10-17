//! In-app update check and install.
//!
//! Stable builds read `latest.json` from the latest GitHub release. The
//! pre-release channel (on by default for a nightly build) walks the release
//! list and also offers the rolling `nightly` tag, compared by publish time.
//! A download is installed only when its SHA-256 matches `latest.json`.
//! Nightly reads that file from the `nightly` release, the same way a
//! pre-release tag does. A missing file or an empty checksum is not installed.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
};
use navidog_core::settings::Settings;
use navidog_ui::tokens;
use navidog_util::update::{hex_encode, newer_version, nightly_is_newer, notes_preview};
use rust_i18n::t;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const MANIFEST_URL: &str = "https://github.com/xhofe/navidog/releases/latest/download/latest.json";
const LATEST_API: &str = "https://api.github.com/repos/xhofe/navidog/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/xhofe/navidog/releases/latest";
const RELEASE_LIST_API: &str = "https://api.github.com/repos/xhofe/navidog/releases?per_page=15";
const NIGHTLY_TAG: &str = "nightly";
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
const USER_AGENT: &str = concat!("navidog/", env!("CARGO_PKG_VERSION"));

pub const BUILD_TIMESTAMP: &str = env!("NAVIDOG_BUILD_TIMESTAMP");
pub const BUILD_CHANNEL: &str = env!("NAVIDOG_BUILD_CHANNEL");

pub fn is_nightly_build() -> bool {
    BUILD_CHANNEL == "nightly"
}

pub fn is_smoke_test() -> bool {
    std::env::var("NAVIDOG_SMOKE_TEST").is_ok_and(|v| v == "1")
}

pub fn smoke_gate_is_window() -> bool {
    std::env::var("NAVIDOG_SMOKE_GATE").is_ok_and(|v| v == "window")
}

/// Exits 2 if no frame (or the window gate) succeeds within 30 seconds.
pub fn arm_smoke_watchdog() {
    if !is_smoke_test() {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        eprintln!("NAVIDOG_SMOKE_TIMEOUT: no frame painted within 30s");
        std::process::exit(2);
    });
}

/// Hooks the open window: exit 0 on the first frame, or after the window has
/// stayed up for 5 seconds when `NAVIDOG_SMOKE_GATE=window`.
pub fn arm_smoke_window(window: &mut Window) {
    if !is_smoke_test() {
        return;
    }
    println!("NAVIDOG_SMOKE_WINDOW");
    if smoke_gate_is_window() {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(5));
            println!("NAVIDOG_SMOKE_OK (window gate)");
            std::process::exit(0);
        });
    }
    window.on_next_frame(|_, _| {
        println!("NAVIDOG_SMOKE_OK");
        std::process::exit(0);
    });
}

#[derive(Clone, Debug)]
pub struct UpdateAsset {
    pub url: String,
    pub sha256: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
    pub page_url: String,
    pub notes: String,
    pub asset: Option<UpdateAsset>,
}

pub(crate) enum Availability {
    Available(UpdateInfo),
    Current,
    Unpublished,
}

/// The release list either has nothing published, nothing newer, or one release to offer.
enum Listed<'a> {
    Unpublished,
    Current,
    Chosen(&'a GithubRelease),
}

pub enum Delivery {
    /// The running `.app` was replaced. Restart to use it.
    #[cfg(target_os = "macos")]
    Replaced,
    /// The installer was handed to the operating system.
    HandedToOs,
}

/// Why a download was not installed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InstallError {
    /// The checksum was missing or did not match.
    Checksum,
    Other(String),
}

#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    assets: Vec<ManifestAsset>,
}

#[derive(Debug, Deserialize)]
struct ManifestAsset {
    os: String,
    arch: String,
    kind: String,
    name: String,
    url: String,
    #[serde(default)]
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    published_at: String,
}

fn pick_asset_for(assets: &[ManifestAsset], os: &str, arch: &str) -> Option<UpdateAsset> {
    let preferred = match os {
        "macos" => "dmg",
        "windows" => "msi",
        "linux" => "appimage",
        _ => return None,
    };
    let mut here = assets.iter().filter(|a| a.os == os && a.arch == arch);
    let chosen = here
        .clone()
        .find(|a| a.kind == preferred)
        .or_else(|| here.find(|_| true))?;
    Some(UpdateAsset {
        url: chosen.url.clone(),
        sha256: chosen.sha256.clone(),
        name: chosen.name.clone(),
    })
}

/// The asset for this machine from a release's `latest.json`. `None` when the
/// file is missing or the asset has no checksum: those are not installed.
fn release_manifest_asset(tag: &str) -> Option<UpdateAsset> {
    let url = format!("https://github.com/xhofe/navidog/releases/download/{tag}/latest.json");
    let text = http_get(&url, Duration::from_secs(15)).ok()?;
    let manifest = serde_json::from_str::<Manifest>(&text).ok()?;
    let asset = pick_asset_for(
        &manifest.assets,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )?;
    (!asset.sha256.trim().is_empty()).then_some(asset)
}

fn current_label() -> String {
    let version = env!("CARGO_PKG_VERSION");
    if is_nightly_build() {
        format!(
            "{version} (nightly {})",
            BUILD_TIMESTAMP.get(..10).unwrap_or("")
        )
    } else {
        version.to_string()
    }
}

/// A 404 from `/releases/latest` means GitHub has no published release to offer.
/// Any other failure stays an error, with `detail` for the notification.
fn missing_latest(status: Option<u16>, detail: &str) -> Result<Availability, String> {
    if status == Some(404) {
        Ok(Availability::Unpublished)
    } else {
        Err(detail.to_string())
    }
}

/// First non-draft release that is newer than this build. Drafts do not count as published.
fn pick_release(releases: &[GithubRelease]) -> Listed<'_> {
    let mut published = false;
    for release in releases.iter().filter(|release| !release.draft) {
        published = true;
        if release.tag_name == NIGHTLY_TAG {
            if nightly_is_newer(&release.published_at, BUILD_TIMESTAMP) {
                return Listed::Chosen(release);
            }
            continue;
        }
        if newer_version(&release.tag_name, env!("CARGO_PKG_VERSION")).is_some() {
            return Listed::Chosen(release);
        }
    }
    if published {
        Listed::Current
    } else {
        Listed::Unpublished
    }
}

/// Blocking. Run it off the UI thread.
pub fn fetch_latest_release(include_prerelease: bool) -> Result<Availability, String> {
    if include_prerelease {
        match fetch_from_release_list() {
            Ok(found) => return Ok(found),
            Err(e) => tracing::debug!(error = %e, "update check: release list unavailable"),
        }
    }
    match fetch_from_manifest() {
        Ok(Some(info)) => return Ok(Availability::Available(info)),
        Ok(None) => return Ok(Availability::Current),
        Err(e) => tracing::debug!(error = %e, "update check: manifest unavailable"),
    }
    fetch_from_api()
}

fn fetch_from_manifest() -> Result<Option<UpdateInfo>, String> {
    let manifest: Manifest = serde_json::from_str(
        &http_get(MANIFEST_URL, Duration::from_secs(15)).map_err(|err| err.detail)?,
    )
    .map_err(|e| e.to_string())?;
    let Some(latest) = newer_version(&manifest.version, env!("CARGO_PKG_VERSION")) else {
        return Ok(None);
    };
    let page_url = if manifest.notes.starts_with("http") {
        manifest.notes.clone()
    } else {
        RELEASES_PAGE.to_string()
    };
    Ok(Some(UpdateInfo {
        notes: release_notes_for(&latest),
        version: latest,
        current: current_label(),
        page_url,
        asset: pick_asset_for(
            &manifest.assets,
            std::env::consts::OS,
            std::env::consts::ARCH,
        ),
    }))
}

fn release_notes_for(version: &str) -> String {
    let Ok(text) = http_get(LATEST_API, Duration::from_secs(15)) else {
        return String::new();
    };
    let Ok(release) = serde_json::from_str::<GithubRelease>(&text) else {
        return String::new();
    };
    if release.tag_name.trim_start_matches('v') == version {
        release.body.trim().to_string()
    } else {
        String::new()
    }
}

fn fetch_from_api() -> Result<Availability, String> {
    let text = match http_get(LATEST_API, Duration::from_secs(15)) {
        Ok(text) => text,
        Err(err) => return missing_latest(err.status, &err.detail),
    };
    let release: GithubRelease = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if release.draft || release.prerelease {
        return Ok(Availability::Current);
    }
    let Some(latest) = newer_version(&release.tag_name, env!("CARGO_PKG_VERSION")) else {
        return Ok(Availability::Current);
    };
    let page_url = if release.html_url.is_empty() {
        RELEASES_PAGE.to_string()
    } else {
        release.html_url
    };
    Ok(Availability::Available(UpdateInfo {
        version: latest,
        current: current_label(),
        page_url,
        notes: release.body.trim().to_string(),
        asset: None,
    }))
}

fn fetch_from_release_list() -> Result<Availability, String> {
    let releases: Vec<GithubRelease> = serde_json::from_str(
        &http_get(RELEASE_LIST_API, Duration::from_secs(15)).map_err(|err| err.detail)?,
    )
    .map_err(|e| e.to_string())?;
    let release = match pick_release(&releases) {
        Listed::Unpublished => return Ok(Availability::Unpublished),
        Listed::Current => return Ok(Availability::Current),
        Listed::Chosen(release) => release,
    };
    let page_url = if release.html_url.trim().is_empty() {
        RELEASES_PAGE.to_string()
    } else {
        release.html_url.clone()
    };
    let asset = release_manifest_asset(&release.tag_name);
    if release.tag_name == NIGHTLY_TAG {
        let published = release.published_at.get(..10).unwrap_or(NIGHTLY_TAG);
        return Ok(Availability::Available(UpdateInfo {
            version: format!("{NIGHTLY_TAG} {published}"),
            current: current_label(),
            page_url,
            notes: release.body.trim().to_string(),
            asset,
        }));
    }
    let Some(latest) = newer_version(&release.tag_name, env!("CARGO_PKG_VERSION")) else {
        return Ok(Availability::Current);
    };
    Ok(Availability::Available(UpdateInfo {
        version: latest,
        current: current_label(),
        page_url,
        notes: release.body.trim().to_string(),
        asset,
    }))
}

struct HttpFail {
    status: Option<u16>,
    detail: String,
}

fn http_get(url: &str, timeout: Duration) -> Result<String, HttpFail> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|err| HttpFail {
            status: match &err {
                ureq::Error::StatusCode(code) => Some(*code),
                _ => None,
            },
            detail: err.to_string(),
        })?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|err| HttpFail {
            status: None,
            detail: err.to_string(),
        })
}

/// Last path segment of a download name. Rejects empty names and `..`.
fn download_file_name(name: &str) -> Option<&str> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    if name.is_empty() || name == "." || name == ".." {
        None
    } else {
        Some(name)
    }
}

/// Downloads `asset` into the update directory and checks its SHA-256.
pub fn download_verified(asset: &UpdateAsset) -> Result<PathBuf, InstallError> {
    if asset.sha256.trim().is_empty() {
        return Err(InstallError::Checksum);
    }
    let Some(name) = download_file_name(&asset.name) else {
        return Err(InstallError::Other("download name is empty".into()));
    };
    let dir = std::env::temp_dir().join("navidog-update");
    std::fs::create_dir_all(&dir).map_err(|e| InstallError::Other(e.to_string()))?;
    let path = dir.join(name);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into();
    let mut response = agent
        .get(&asset.url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| InstallError::Other(e.to_string()))?;
    let mut reader = response.body_mut().as_reader();
    let mut file = std::fs::File::create(&path).map_err(|e| InstallError::Other(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| InstallError::Other(e.to_string()))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_DOWNLOAD {
            let _ = std::fs::remove_file(&path);
            return Err(InstallError::Other("download is larger than 512 MB".into()));
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| InstallError::Other(e.to_string()))?;
    }
    let got = hex_encode(&hasher.finalize());
    if !got.eq_ignore_ascii_case(asset.sha256.trim()) {
        let _ = std::fs::remove_file(&path);
        return Err(InstallError::Checksum);
    }
    Ok(path)
}

fn cleanup_download(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent()
        && dir.file_name().is_some_and(|name| name == "navidog-update")
    {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(any(test, target_os = "macos"))]
pub fn bundle_from_exe(path: &Path) -> Option<PathBuf> {
    let mut cursor = path.to_path_buf();
    loop {
        if cursor.extension().is_some_and(|ext| ext == "app") {
            return Some(cursor);
        }
        if !cursor.pop() {
            return None;
        }
    }
}

pub fn install(path: &Path) -> Result<Delivery, String> {
    #[cfg(target_os = "macos")]
    {
        install_macos(path)
    }
    #[cfg(target_os = "windows")]
    {
        install_windows(path)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        install_other(path)
    }
}

#[cfg(target_os = "macos")]
fn install_macos(dmg: &Path) -> Result<Delivery, String> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(app) = bundle_from_exe(&exe)
        && app
            .parent()
            .is_some_and(|dir| std::fs::write(dir.join(".navidog-update-probe"), b"").is_ok())
    {
        let _ = app.parent().map(|dir| {
            let _ = std::fs::remove_file(dir.join(".navidog-update-probe"));
        });
        if replace_bundle(&app, dmg).is_ok() {
            return Ok(Delivery::Replaced);
        }
    }
    std::process::Command::new("open")
        .arg(dmg)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(Delivery::HandedToOs)
}

#[cfg(target_os = "macos")]
fn replace_bundle(app: &Path, dmg: &Path) -> Result<(), String> {
    let mount = std::env::temp_dir().join(format!("navidog-dmg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&mount);
    std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
    let attached = std::process::Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
        .arg(&mount)
        .arg(dmg)
        .status()
        .map_err(|e| e.to_string())?;
    if !attached.success() {
        return Err("could not mount the disk image".into());
    }
    let src = std::fs::read_dir(&mount)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "app"));
    let Some(src) = src else {
        let _ = std::process::Command::new("hdiutil")
            .args(["detach"])
            .arg(&mount)
            .status();
        return Err("the disk image has no app".into());
    };
    let staged = std::env::temp_dir().join(format!("navidog-new-{}.app", std::process::id()));
    let _ = std::fs::remove_dir_all(&staged);
    let copied = std::process::Command::new("ditto")
        .arg(&src)
        .arg(&staged)
        .status();
    let _ = std::process::Command::new("hdiutil")
        .args(["detach"])
        .arg(&mount)
        .status();
    match copied {
        Ok(status) if status.success() => {}
        _ => {
            let _ = std::fs::remove_dir_all(&staged);
            return Err("could not copy the new app into place".into());
        }
    }
    let aside = std::env::temp_dir().join(format!("navidog-old-{}.app", std::process::id()));
    let _ = std::fs::remove_dir_all(&aside);
    swap_app(app, &staged, &aside)
}

/// Moves `staged` onto `app`, keeping `aside` until that succeeds. A partial `app`
/// left by a failed copy is removed before the previous bundle is moved back.
#[cfg(any(test, target_os = "macos"))]
fn swap_app(app: &Path, staged: &Path, aside: &Path) -> Result<(), String> {
    if let Err(e) = std::fs::rename(app, aside) {
        let _ = std::fs::remove_dir_all(staged);
        return Err(e.to_string());
    }
    if let Err(e) = std::fs::rename(staged, app) {
        restore_previous_app(app, aside)?;
        return Err(format!("could not copy the new app into place: {e}"));
    }
    if let Err(e) = std::fs::remove_dir_all(aside) {
        tracing::error!("could not remove the previous app: {e}");
    }
    Ok(())
}

#[cfg(any(test, target_os = "macos"))]
fn restore_previous_app(app: &Path, aside: &Path) -> Result<(), String> {
    if app.exists() {
        std::fs::remove_dir_all(app).map_err(|e| {
            tracing::error!("could not remove the partial app: {e}");
            e.to_string()
        })?;
    }
    std::fs::rename(aside, app).map_err(|e| {
        tracing::error!("could not restore the previous app: {e}");
        format!("could not restore the previous app: {e}")
    })
}

/// Shell command that starts the new bundle after this process has exited.
/// `open` without waiting would only activate the process that is still running.
#[cfg(any(test, target_os = "macos"))]
fn relaunch_command(pid: u32, app: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg(
            "nohup sh -c 'while kill -0 \"$PID\" 2>/dev/null; do sleep 0.2; done; exec open \"$APP\"' >/dev/null 2>&1 &",
        )
        .env("PID", pid.to_string())
        .env("APP", app)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd
}

#[cfg(target_os = "macos")]
pub fn relaunch() {
    if let Ok(exe) = std::env::current_exe()
        && let Some(app) = bundle_from_exe(&exe)
    {
        let _ = relaunch_command(std::process::id(), &app).spawn();
    }
}

#[cfg(target_os = "windows")]
fn install_windows(path: &Path) -> Result<Delivery, String> {
    // One argument so cmd does not split a path that contains spaces.
    std::process::Command::new("cmd")
        .arg("/C")
        .arg(format!("start \"\" \"{}\"", path.display()))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(Delivery::HandedToOs)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn install_other(path: &Path) -> Result<Delivery, String> {
    let open = if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".AppImage.tar.gz"))
    {
        extract_appimage(path)?
    } else {
        path.to_path_buf()
    };
    std::process::Command::new("xdg-open")
        .arg(&open)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(Delivery::HandedToOs)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn extract_appimage(archive: &Path) -> Result<PathBuf, String> {
    let dir = archive.with_extension("").with_extension("");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("tar")
        .args(["-xzf"])
        .arg(archive)
        .arg("-C")
        .arg(&dir)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("could not unpack the AppImage".into());
    }
    let appimage = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "AppImage"))
        .ok_or_else(|| "the archive has no AppImage".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&appimage, std::fs::Permissions::from_mode(0o755));
    }
    Ok(appimage)
}

/// When a check reached GitHub. A transport error leaves this `None`, so the
/// next launch can try again instead of waiting two days.
fn checked_at(found: &Result<Availability, String>, now: i64) -> Option<i64> {
    found.is_ok().then_some(now)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Startup check: quiet, and at most once every two days.
pub fn startup_check(window: &mut Window, cx: &mut App) {
    if is_smoke_test() || !Settings::global(cx).update_check_due(unix_now()) {
        return;
    }
    begin_check(false, window, cx);
}

/// Menu item: always checks, and reports up to date, no release, and errors.
pub fn manual_check(window: &mut Window, cx: &mut App) {
    begin_check(true, window, cx);
}

fn begin_check(manual: bool, window: &mut Window, cx: &mut App) {
    let include = Settings::global(cx).include_prerelease(is_nightly_build());
    // Keep this window. The menu callback often runs while `NSApp.mainWindow` is nil,
    // so looking up the active window again after the request would drop the result.
    let home = window.window_handle();
    cx.spawn(async move |cx| {
        let found = cx
            .background_spawn(async move { fetch_latest_release(include) })
            .await;
        cx.update(|cx| {
            let _ = home.update(cx, |_, window, cx| show_check(found, manual, window, cx));
        });
    })
    .detach();
}

fn show_check(
    found: Result<Availability, String>,
    manual: bool,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(at) = checked_at(&found, unix_now()) {
        Settings::update(cx, |s| s.update_checked_unix = Some(at));
    }
    match found {
        Ok(Availability::Available(info)) => open_dialog(info, window, cx),
        Ok(Availability::Current) if manual => {
            window.push_notification(t!("update.uptodate").to_string(), cx);
        }
        Ok(Availability::Unpublished) if manual => {
            window.push_notification(t!("update.unpublished").to_string(), cx);
        }
        Err(detail) if manual => {
            window.push_notification(t!("update.failed", detail = detail).to_string(), cx);
        }
        _ => {}
    }
}

fn open_dialog(info: UpdateInfo, window: &mut Window, cx: &mut App) {
    let view = cx.new(|_| UpdateDialog { info, busy: false });
    let version = view.read(cx).info.version.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
            .title(t!("update.available", version = version).to_string())
            .child(view.clone())
    });
}

struct UpdateDialog {
    info: UpdateInfo,
    busy: bool,
}

impl UpdateDialog {
    fn download(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>) {
        let Some(asset) = self.info.asset.clone() else {
            cx.open_url(&self.info.page_url);
            window.close_dialog(cx);
            return;
        };
        self.busy = true;
        cx.notify();
        window.push_notification(t!("update.downloading").to_string(), cx);
        let page = self.info.page_url.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let path = download_verified(&asset)?;
                    match install(&path) {
                        Ok(delivery) => {
                            #[cfg(target_os = "macos")]
                            if matches!(delivery, Delivery::Replaced) {
                                cleanup_download(&path);
                            }
                            Ok(delivery)
                        }
                        Err(detail) => {
                            cleanup_download(&path);
                            Err(InstallError::Other(detail))
                        }
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    #[cfg(target_os = "macos")]
                    Ok(Delivery::Replaced) => {
                        window.close_dialog(cx);
                        open_restart(window, cx);
                    }
                    Ok(Delivery::HandedToOs) => {
                        window.push_notification(t!("update.handed_off").to_string(), cx);
                        window.close_dialog(cx);
                    }
                    Err(InstallError::Checksum) => {
                        window.push_notification(t!("update.bad_checksum").to_string(), cx);
                        cx.open_url(&page);
                    }
                    Err(InstallError::Other(detail)) => {
                        window.push_notification(
                            t!("update.install_failed", detail = detail).to_string(),
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }
}

impl Render for UpdateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        let notes = notes_preview(&self.info.notes);
        let has_asset = self.info.asset.is_some();
        let page = self.info.page_url.clone();
        let weak = cx.entity().downgrade();
        v_flex()
            .gap_2()
            .w_full()
            .min_w_0()
            .child(t!("update.current", version = self.info.current.clone()).to_string())
            .when(!notes.is_empty(), |col| col.child(notes))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .w_full()
                    .child(
                        Button::new("update-later")
                            .label(t!("update.later").to_string())
                            .disabled(self.busy)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("update-go")
                            .primary()
                            .disabled(self.busy)
                            .label(
                                if has_asset {
                                    t!("update.download")
                                } else {
                                    t!("update.open_page")
                                }
                                .to_string(),
                            )
                            .on_click(move |_, window, cx| {
                                if has_asset {
                                    let _ = weak.update(cx, |this, cx| this.download(window, cx));
                                } else {
                                    cx.open_url(&page);
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    }
}

#[cfg(target_os = "macos")]
fn open_restart(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, window, _| {
        dialog
            .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
            .title(t!("update.restart_body").to_string())
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("restart-later")
                            .label(t!("update.restart_later").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("restart-now")
                            .primary()
                            .label(t!("update.restart_now").to_string())
                            .on_click(|_, _, cx| {
                                #[cfg(target_os = "macos")]
                                relaunch();
                                cx.quit();
                            }),
                    ),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(os: &str, arch: &str, kind: &str, name: &str) -> ManifestAsset {
        ManifestAsset {
            os: os.into(),
            arch: arch.into(),
            kind: kind.into(),
            name: name.into(),
            url: format!("https://example.test/{name}"),
            sha256: "abc".into(),
        }
    }

    #[test]
    fn picks_the_preferred_kind_for_the_machine() {
        let assets = vec![
            asset("linux", "x86_64", "deb", "a.deb"),
            asset("linux", "x86_64", "appimage", "a.AppImage.tar.gz"),
            asset("linux", "aarch64", "appimage", "b.AppImage.tar.gz"),
        ];
        assert_eq!(
            pick_asset_for(&assets, "linux", "x86_64").unwrap().name,
            "a.AppImage.tar.gz"
        );
        assert!(pick_asset_for(&assets, "macos", "aarch64").is_none());
    }

    #[test]
    fn no_published_release_is_not_treated_as_up_to_date() {
        assert!(matches!(pick_release(&[]), Listed::Unpublished));
        let drafts: Vec<GithubRelease> =
            serde_json::from_str(r#"[{"tag_name":"9.0.0","draft":true}]"#).unwrap();
        assert!(matches!(pick_release(&drafts), Listed::Unpublished));
        let older: Vec<GithubRelease> = serde_json::from_str(r#"[{"tag_name":"0.0.0"}]"#).unwrap();
        assert!(matches!(pick_release(&older), Listed::Current));
        let newer: Vec<GithubRelease> = serde_json::from_str(r#"[{"tag_name":"9.0.0"}]"#).unwrap();
        assert!(matches!(pick_release(&newer), Listed::Chosen(_)));
        assert!(matches!(
            missing_latest(Some(404), "http status: 404"),
            Ok(Availability::Unpublished)
        ));
        assert!(missing_latest(None, "connection failed").is_err());
    }

    #[test]
    fn a_running_app_bundle_is_the_nearest_app_ancestor() {
        let path = Path::new("/Applications/Navidog.app/Contents/MacOS/navidog");
        assert_eq!(
            bundle_from_exe(path).unwrap(),
            PathBuf::from("/Applications/Navidog.app")
        );
        assert!(bundle_from_exe(Path::new("/usr/local/bin/navidog")).is_none());
    }

    #[test]
    fn a_failed_check_is_not_recorded() {
        assert_eq!(checked_at(&Ok(Availability::Current), 10), Some(10));
        assert_eq!(checked_at(&Err("offline".into()), 10), None);
    }

    #[test]
    fn download_name_stays_inside_the_update_directory() {
        assert_eq!(download_file_name("../../etc/passwd"), Some("passwd"));
        assert_eq!(download_file_name("navidog.dmg"), Some("navidog.dmg"));
        assert_eq!(download_file_name(".."), None);
        assert_eq!(download_file_name(""), None);
    }

    fn serve(body: &'static [u8]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = std::io::Read::read(&mut sock, &mut buf);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut sock, header.as_bytes());
            let _ = std::io::Write::write_all(&mut sock, body);
        });
        format!("http://{addr}/pkg.dmg")
    }

    #[test]
    fn download_verified_checks_the_checksum_and_the_file_name() {
        let empty = UpdateAsset {
            url: "http://127.0.0.1/missing".into(),
            sha256: "  ".into(),
            name: "navidog.dmg".into(),
        };
        assert_eq!(download_verified(&empty), Err(InstallError::Checksum));

        let body: &'static [u8] = b"navidog-update";
        let good = hex_encode(&Sha256::digest(body));
        let url = serve(body);
        let wrong = UpdateAsset {
            url: url.clone(),
            sha256: "deadbeef".into(),
            name: format!("../../navidog-bad-{}.dmg", std::process::id()),
        };
        assert_eq!(download_verified(&wrong), Err(InstallError::Checksum));
        let bad_name = download_file_name(&wrong.name).unwrap();
        assert!(
            !std::env::temp_dir()
                .join("navidog-update")
                .join(bad_name)
                .exists()
        );

        let url = serve(body);
        let name = format!("../../navidog-ok-{}.dmg", std::process::id());
        let right = UpdateAsset {
            url,
            sha256: good,
            name: name.clone(),
        };
        let path = download_verified(&right).unwrap();
        let file_name = download_file_name(&name).unwrap();
        assert_eq!(
            path,
            std::env::temp_dir().join("navidog-update").join(file_name)
        );
        assert!(!path.components().any(|c| c.as_os_str() == ".."));
        assert_eq!(std::fs::read(&path).unwrap(), body);
        cleanup_download(&path);
    }

    #[test]
    fn relaunch_waits_until_this_process_exits() {
        let cmd = relaunch_command(42, Path::new("/Applications/Navidog.app"));
        let script = cmd
            .get_args()
            .nth(1)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(script.contains("kill -0"));
        assert!(script.contains("exec open"));
        assert!(!script.contains("-n"));
        let env: Vec<_> = cmd.get_envs().collect();
        assert!(env.iter().any(|(k, v)| *k == "PID" && v.unwrap() == "42"));
        assert!(
            env.iter()
                .any(|(k, v)| { *k == "APP" && v.unwrap() == "/Applications/Navidog.app" })
        );
    }

    #[test]
    fn a_failed_swap_restores_the_previous_app() {
        let dir = std::env::temp_dir().join(format!("navidog-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let app = dir.join("Navidog.app");
        let staged = dir.join("Navidog.app.new");
        let aside = dir.join("aside.app");
        std::fs::create_dir(&app).unwrap();
        std::fs::write(app.join("marker"), b"old").unwrap();
        let err = swap_app(&app, &staged, &aside).unwrap_err();
        assert!(err.contains("could not copy"), "{err}");
        assert_eq!(std::fs::read(app.join("marker")).unwrap(), b"old");
        assert!(!aside.exists());

        std::fs::create_dir_all(&aside).unwrap();
        std::fs::write(aside.join("marker"), b"old").unwrap();
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("marker"), b"partial").unwrap();
        restore_previous_app(&app, &aside).unwrap();
        assert_eq!(std::fs::read(app.join("marker")).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
