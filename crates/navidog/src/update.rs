//! In-app update check and install.
//!
//! Both channels read `latest.json` from [`RELEASE_MIRROR`]. The file's
//! `releases` list is every published release; the top-level fields stay this
//! release's own version and assets. The stable channel skips pre-releases and
//! the rolling `nightly` tag. Nightly is compared by publish time. A download
//! is installed only when its SHA-256 matches the manifest. A missing file or
//! an empty checksum is not installed.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
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

/// Release files. This mirror only serves the `xhofe` user.
const RELEASE_MIRROR: &str = "https://gh.nn.ci";
const REPO: &str = "xhofe/navidog";

/// Nightly's manifest. Its `releases` list covers both channels.
fn manifest_url() -> String {
    format!("{RELEASE_MIRROR}/{REPO}/releases/download/nightly/latest.json")
}

/// Used when the nightly release does not exist yet.
fn manifest_url_stable() -> String {
    format!("{RELEASE_MIRROR}/{REPO}/releases/latest/download/latest.json")
}

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
    /// Advertised size from `latest.json`. `0` when the manifest omitted it.
    pub size: u64,
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
#[derive(Debug)]
enum Listed<'a> {
    Unpublished,
    Current,
    Chosen(&'a UpdateRelease),
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
    /// The user cancelled before the installer was applied.
    Cancelled,
    Other(String),
}

/// `latest.json`. `releases` is the index; the other fields are this release.
/// An older file has no `releases` and is treated as that one release.
#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    version: String,
    #[serde(default)]
    tag: String,
    #[serde(default, deserialize_with = "empty_if_null")]
    body: String,
    #[serde(default)]
    assets: Vec<ManifestAsset>,
    #[serde(default)]
    releases: Vec<UpdateRelease>,
}

/// One entry of `latest.json`'s `releases`. `body` is null when there are no notes.
#[derive(Debug, Deserialize)]
struct UpdateRelease {
    tag: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default, deserialize_with = "empty_if_null")]
    published_at: String,
    #[serde(default, deserialize_with = "empty_if_null")]
    body: String,
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
    #[serde(default)]
    size: u64,
}

fn empty_if_null<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// The browser page for a tag. The file mirror does not serve this page.
fn release_page(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/{tag}")
}

/// Rewrite a GitHub release file for this repo onto the file mirror.
/// Another host, including an address that is already mirrored, stays as it is.
fn mirror_release_file(url: &str) -> String {
    let github = format!("https://github.com/{REPO}/");
    match url.strip_prefix(github.as_str()) {
        Some(rest) => format!("{RELEASE_MIRROR}/{REPO}/{rest}"),
        None => url.to_string(),
    }
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
        url: mirror_release_file(&chosen.url),
        sha256: chosen.sha256.clone(),
        name: chosen.name.clone(),
        size: chosen.size,
    })
}

/// The installer for this machine. `None` when the release has none, or the
/// checksum is empty: those are not installed. The dialog then offers the page.
fn installer_for(release: &UpdateRelease) -> Option<UpdateAsset> {
    let Some(asset) = pick_asset_for(
        &release.assets,
        std::env::consts::OS,
        std::env::consts::ARCH,
    ) else {
        tracing::warn!(
            tag = %release.tag,
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            "update: no installer for this computer"
        );
        return None;
    };
    if asset.sha256.trim().is_empty() {
        tracing::warn!(tag = %release.tag, "update: installer has no checksum");
        return None;
    }
    Some(asset)
}

/// What About and the update dialog call this build.
///
/// A nightly shows the channel and the UTC build time baked by CI, which is the
/// same clock as the GitHub nightly notes. It does not show the Cargo.toml
/// version. A stable build shows the version compiled in, which a tag build
/// stamps from the tag before compiling.
pub(crate) fn current_label() -> String {
    version_label(env!("CARGO_PKG_VERSION"), BUILD_CHANNEL, BUILD_TIMESTAMP)
}

pub(crate) fn version_label(version: &str, channel: &str, built_at: &str) -> String {
    if channel == "nightly" {
        let date = built_at.get(..10).unwrap_or("");
        let time = built_at.get(11..16).unwrap_or("");
        format!("nightly ({date} {time} UTC)")
    } else {
        version.to_string()
    }
}

/// A 404 from both index URLs means nothing has been published.
/// Any other failure stays an error, with `detail` for the notification.
fn missing_latest(status: Option<u16>, detail: &str) -> Result<Availability, String> {
    if status == Some(404) {
        Ok(Availability::Unpublished)
    } else {
        Err(detail.to_string())
    }
}

/// Newest published release that this build should offer.
///
/// Entries are ordered by `published_at`, newest first. Drafts do not count as
/// published. Without `include_prerelease`, pre-releases and `nightly` are skipped.
fn pick_release(releases: &[UpdateRelease], include_prerelease: bool) -> Listed<'_> {
    let mut order: Vec<usize> = (0..releases.len()).collect();
    order.sort_by(|&a, &b| releases[b].published_at.cmp(&releases[a].published_at));
    let mut published = false;
    for release in order.iter().map(|&index| &releases[index]) {
        if release.draft {
            continue;
        }
        if !include_prerelease && (release.prerelease || release.tag == NIGHTLY_TAG) {
            continue;
        }
        published = true;
        if release.tag == NIGHTLY_TAG {
            if nightly_is_newer(&release.published_at, BUILD_TIMESTAMP) {
                return Listed::Chosen(release);
            }
            continue;
        }
        let version = if release.version.is_empty() {
            release.tag.as_str()
        } else {
            release.version.as_str()
        };
        if newer_version(version, env!("CARGO_PKG_VERSION")).is_some() {
            return Listed::Chosen(release);
        }
    }
    if published {
        Listed::Current
    } else {
        Listed::Unpublished
    }
}

fn offered_version(release: &UpdateRelease) -> String {
    if release.tag == NIGHTLY_TAG {
        let published = release.published_at.get(..10).unwrap_or(NIGHTLY_TAG);
        return format!("{NIGHTLY_TAG} {published}");
    }
    let raw = if release.version.is_empty() {
        release.tag.as_str()
    } else {
        release.version.as_str()
    };
    newer_version(raw, env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|| raw.trim_start_matches('v').to_string())
}

/// Blocking. Run it off the UI thread.
pub fn fetch_latest_release(include_prerelease: bool) -> Result<Availability, String> {
    let text = match read_updates() {
        Ok(text) => text,
        Err(err) => return missing_latest(err.status, &err.detail),
    };
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let releases = listed_releases(manifest);
    let release = match pick_release(&releases, include_prerelease) {
        Listed::Unpublished => return Ok(Availability::Unpublished),
        Listed::Current => return Ok(Availability::Current),
        Listed::Chosen(release) => release,
    };
    Ok(Availability::Available(UpdateInfo {
        version: offered_version(release),
        current: current_label(),
        page_url: release_page(&release.tag),
        notes: release.body.trim().to_string(),
        asset: installer_for(release),
    }))
}

fn read_updates() -> Result<String, HttpFail> {
    match http_get(&manifest_url(), Duration::from_secs(15)) {
        Ok(text) => Ok(text),
        Err(err) if err.status == Some(404) => {
            http_get(&manifest_url_stable(), Duration::from_secs(15))
        }
        Err(err) => Err(err),
    }
}

/// The index, or this file alone when it was published before `releases` existed.
fn listed_releases(manifest: Manifest) -> Vec<UpdateRelease> {
    if !manifest.releases.is_empty() {
        return manifest.releases;
    }
    if manifest.tag.is_empty() && manifest.version.is_empty() {
        return Vec::new();
    }
    vec![UpdateRelease {
        tag: manifest.tag,
        version: manifest.version,
        prerelease: false,
        draft: false,
        published_at: String::new(),
        body: manifest.body,
        assets: manifest.assets,
    }]
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
///
/// `on_progress(downloaded, total)` runs as bytes arrive. `total` is the
/// response `Content-Length` when it is present, otherwise `asset.size`, and
/// `0` when neither is known.
///
/// ponytail: `cancel` is noticed between chunks. The connect and each blocking
/// read run to completion; aborting an in-flight read means replacing this ureq loop.
pub fn download_verified(
    asset: &UpdateAsset,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<PathBuf, InstallError> {
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
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(asset.size);
    on_progress(0, total);
    let mut reader = response.body_mut().as_reader();
    let mut file = std::fs::File::create(&path).map_err(|e| InstallError::Other(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(InstallError::Cancelled);
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| InstallError::Other(e.to_string()))?;
        if n == 0 {
            break;
        }
        downloaded += n as u64;
        if downloaded > MAX_DOWNLOAD {
            let _ = std::fs::remove_file(&path);
            return Err(InstallError::Other("download is larger than 512 MB".into()));
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| InstallError::Other(e.to_string()))?;
        on_progress(downloaded, total);
    }
    if cancel.load(Ordering::Relaxed) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(InstallError::Cancelled);
    }
    let got = hex_encode(&hasher.finalize());
    if !got.eq_ignore_ascii_case(asset.sha256.trim()) {
        let _ = std::fs::remove_file(&path);
        return Err(InstallError::Checksum);
    }
    Ok(path)
}

/// `0` when the total is unknown. Otherwise a percentage in `0..=100`.
fn download_percent(done: u64, total: u64) -> u64 {
    done.saturating_mul(100)
        .checked_div(total)
        .unwrap_or(0)
        .min(100)
}

/// Repaint when the percentage changes. An unknown total has no percentage, so
/// any new byte count repaints the indeterminate bar's caption.
fn progress_changed(shown: Option<(u64, u64)>, done: u64, total: u64) -> bool {
    let Some((prev_done, prev_total)) = shown else {
        return true;
    };
    if total == 0 {
        done != prev_done
    } else {
        download_percent(done, total) != download_percent(prev_done, prev_total)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let n = bytes as f64;
    let (value, unit) = if n < KIB {
        return format!("{bytes} B");
    } else if n < KIB * KIB {
        (n / KIB, "KB")
    } else if n < KIB * KIB * KIB {
        (n / (KIB * KIB), "MB")
    } else {
        (n / (KIB * KIB * KIB), "GB")
    };
    format!("{value:.1} {unit}")
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
    let view = cx.new(|_| UpdateDialog {
        info,
        progress: None,
        installing: false,
        cancel: Arc::new(AtomicBool::new(false)),
    });
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
    /// `(downloaded, total)` while a download is in flight. `total` is 0 when
    /// the size is unknown. `None` shows the action buttons.
    progress: Option<(u64, u64)>,
    /// The file is on disk and install has started. Cancel no longer applies.
    installing: bool,
    cancel: Arc<AtomicBool>,
}

/// Applies download ticks to the dialog. Percentage-sized steps only, so a
/// 64 KiB chunk does not repaint on its own.
async fn watch_download(
    view: gpui_kit::WeakEntity<UpdateDialog>,
    rx: std::sync::mpsc::Receiver<(u64, u64)>,
    cx: &mut gpui_kit::AsyncWindowContext,
) {
    let mut shown: Option<(u64, u64)> = None;
    loop {
        let mut latest = None;
        let mut closed = false;
        loop {
            match rx.try_recv() {
                Ok(tick) => latest = Some(tick),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        if let Some((done, total)) = latest
            && progress_changed(shown, done, total)
        {
            shown = Some((done, total));
            let _ = view.update_in(cx, |dialog, _, cx| {
                dialog.progress = Some((done, total));
                cx.notify();
            });
        }
        if closed {
            break;
        }
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
    }
}

fn download_progress_bar(done: u64, total: u64) -> impl IntoElement {
    let bar = if total == 0 {
        Progress::new("update-progress").w_full().loading(true)
    } else {
        Progress::new("update-progress")
            .w_full()
            .value(download_percent(done, total) as f32)
    };
    let caption = if total == 0 {
        t!("update.downloading").to_string()
    } else {
        t!(
            "update.downloading_progress",
            percent = download_percent(done, total),
            done = format_bytes(done),
            total = format_bytes(total)
        )
        .to_string()
    };
    v_flex().w_full().gap_2().child(bar).child(caption)
}

impl UpdateDialog {
    fn download(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>) {
        if self.progress.is_some() {
            return;
        }
        let Some(asset) = self.info.asset.clone() else {
            cx.open_url(&self.info.page_url);
            window.close_dialog(cx);
            return;
        };
        self.cancel.store(false, Ordering::Relaxed);
        self.installing = false;
        // Show the bar on the click itself. The first byte waits on DNS and
        // TLS, and a still-enabled button looks like the click did nothing.
        self.progress = Some((0, asset.size));
        cx.notify();
        let page = self.info.page_url.clone();
        let cancel = Arc::clone(&self.cancel);
        let (tx, rx) = std::sync::mpsc::channel();
        cx.spawn_in(window, async move |this, cx| {
            let view = this.clone();
            let watch = cx.spawn(async move |cx| watch_download(view, rx, cx).await);
            let downloaded = cx
                .background_spawn(async move {
                    download_verified(&asset, &cancel, |done, total| {
                        let _ = tx.send((done, total));
                    })
                })
                .await;
            let result = match downloaded {
                Err(err) => Err(err),
                Ok(path) => {
                    let cancelled = this
                        .update_in(cx, |dialog, _, _| dialog.cancel.load(Ordering::Relaxed))
                        .unwrap_or(false);
                    if cancelled {
                        cleanup_download(&path);
                        Err(InstallError::Cancelled)
                    } else {
                        let _ = this.update_in(cx, |dialog, _, cx| {
                            dialog.installing = true;
                            cx.notify();
                        });
                        cx.background_spawn(async move {
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
                        .await
                    }
                }
            };
            watch.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.progress = None;
                this.installing = false;
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
                    Err(InstallError::Cancelled) => {}
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
        let progress = self.progress;
        let installing = self.installing;
        let cancel = Arc::clone(&self.cancel);
        v_flex()
            .gap_2()
            .w_full()
            .min_w_0()
            .child(t!("update.current", version = self.info.current.clone()).to_string())
            .when(!notes.is_empty(), |col| col.child(notes))
            .when(progress.is_none() && !has_asset, |col| {
                col.child(t!("update.no_asset").to_string())
            })
            .when_some(progress, |col, (done, total)| {
                col.child(download_progress_bar(done, total))
                    .when(!installing, |col| {
                        col.child(
                            h_flex().justify_end().w_full().child(
                                Button::new("update-cancel")
                                    .label(t!("dialog.cancel").to_string())
                                    .on_click(move |_, _, _| {
                                        cancel.store(true, Ordering::Relaxed);
                                    }),
                            ),
                        )
                    })
            })
            .when(progress.is_none(), |col| {
                col.child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .w_full()
                        .child(
                            Button::new("update-later")
                                .label(t!("update.later").to_string())
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("update-go")
                                .primary()
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
                                        let _ =
                                            weak.update(cx, |this, cx| this.download(window, cx));
                                    } else {
                                        cx.open_url(&page);
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
            })
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
            size: 0,
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
        assert_eq!(
            pick_asset_for(&assets, "linux", "x86_64").unwrap().url,
            "https://example.test/a.AppImage.tar.gz"
        );
    }

    #[test]
    fn release_files_go_through_the_mirror() {
        assert_eq!(
            manifest_url(),
            "https://gh.nn.ci/xhofe/navidog/releases/download/nightly/latest.json"
        );
        assert_eq!(
            manifest_url_stable(),
            "https://gh.nn.ci/xhofe/navidog/releases/latest/download/latest.json"
        );
        let github =
            "https://github.com/xhofe/navidog/releases/download/v0.2.0/Navidog-aarch64.dmg";
        let mirrored =
            "https://gh.nn.ci/xhofe/navidog/releases/download/v0.2.0/Navidog-aarch64.dmg";
        assert_eq!(mirror_release_file(github), mirrored);
        assert_eq!(mirror_release_file(mirrored), mirrored);
    }

    #[test]
    fn one_list_serves_both_channels() {
        assert!(matches!(pick_release(&[], false), Listed::Unpublished));
        let list: Vec<UpdateRelease> = serde_json::from_str(
            r#"[
                {"tag":"v8.0.0","version":"8.0.0","draft":true,"published_at":null,"body":null},
                {"tag":"nightly","version":"nightly","prerelease":true,"published_at":"2099-01-02T00:00:00Z","body":"nightly notes"},
                {"tag":"v9.0.0","version":"9.0.0","published_at":"2099-01-01T00:00:00Z","body":"nine"},
                {"tag":"v9.1.0-rc.1","version":"9.1.0-rc.1","prerelease":true,"published_at":"2098-01-01T00:00:00Z","body":"rc"},
                {"tag":"v0.0.0","version":"0.0.0","published_at":"2020-01-01T00:00:00Z","body":"old"}
            ]"#,
        )
        .unwrap();
        assert!(list[0].body.is_empty());
        assert!(list[0].published_at.is_empty());
        assert!(matches!(
            pick_release(&list[..1], false),
            Listed::Unpublished
        ));
        match pick_release(&list, false) {
            Listed::Chosen(release) => assert_eq!(release.tag, "v9.0.0"),
            other => panic!("stable channel should offer v9.0.0, got {other:?}"),
        }
        match pick_release(&list, true) {
            Listed::Chosen(release) => assert_eq!(release.tag, "nightly"),
            other => panic!("pre-release channel should offer nightly, got {other:?}"),
        }
        let older: Vec<UpdateRelease> =
            serde_json::from_str(r#"[{"tag":"v0.0.0","version":"0.0.0"}]"#).unwrap();
        assert!(matches!(pick_release(&older, false), Listed::Current));
        let rc: Vec<UpdateRelease> = serde_json::from_str(
            r#"[{"tag":"v9.1.0-rc.1","version":"9.1.0-rc.1","prerelease":true,"published_at":"2099-01-01T00:00:00Z"}]"#,
        )
        .unwrap();
        assert!(matches!(pick_release(&rc, false), Listed::Unpublished));
        match pick_release(&rc, true) {
            Listed::Chosen(release) => assert_eq!(release.tag, "v9.1.0-rc.1"),
            other => panic!("pre-release channel should offer the rc, got {other:?}"),
        }
        assert!(matches!(
            missing_latest(Some(404), "http status: 404"),
            Ok(Availability::Unpublished)
        ));
        assert!(missing_latest(None, "connection failed").is_err());
    }

    #[test]
    fn an_old_latest_json_is_one_release() {
        let manifest: Manifest = serde_json::from_str(
            r#"{"version":"9.0.0","tag":"v9.0.0","notes":"https://github.com/xhofe/navidog/releases/tag/v9.0.0","assets":[]}"#,
        )
        .unwrap();
        let releases = listed_releases(manifest);
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "v9.0.0");
        assert!(matches!(pick_release(&releases, false), Listed::Chosen(_)));
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
    fn nightly_label_names_the_channel_and_the_build_time() {
        let label = version_label("0.0.1", "nightly", "2026-10-17T18:05:00Z");
        assert_eq!(label, "nightly (2026-10-17 18:05 UTC)");
        assert!(!label.contains("0.0.1"));
        assert_eq!(
            version_label("0.2.0", "stable", "2026-10-17T18:05:00Z"),
            "0.2.0"
        );
    }

    #[test]
    fn progress_repaints_when_the_percent_changes() {
        assert!(progress_changed(None, 0, 100));
        assert!(!progress_changed(Some((0, 100)), 0, 100));
        assert!(!progress_changed(Some((1, 1000)), 9, 1000));
        assert!(progress_changed(Some((9, 1000)), 10, 1000));
        assert!(progress_changed(Some((0, 0)), 1, 0));
        assert!(!progress_changed(Some((1, 0)), 1, 0));
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
    fn cancelling_a_download_removes_the_partial_file() {
        let body: &'static [u8] = b"navidog-update-cancel";
        let url = serve(body);
        let name = format!("navidog-cancel-{}.dmg", std::process::id());
        let asset = UpdateAsset {
            url,
            sha256: hex_encode(&Sha256::digest(body)),
            name: name.clone(),
            size: body.len() as u64,
        };
        let cancel = AtomicBool::new(false);
        let err = download_verified(&asset, &cancel, |done, _| {
            if done == 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert_eq!(err, Err(InstallError::Cancelled));
        let path = std::env::temp_dir().join("navidog-update").join(&name);
        assert!(!path.exists(), "{}", path.display());
    }

    #[test]
    fn download_verified_checks_the_checksum_and_the_file_name() {
        let empty = UpdateAsset {
            url: "http://127.0.0.1/missing".into(),
            sha256: "  ".into(),
            name: "navidog.dmg".into(),
            size: 0,
        };
        assert_eq!(
            download_verified(&empty, &AtomicBool::new(false), |_, _| {}),
            Err(InstallError::Checksum)
        );

        let body: &'static [u8] = b"navidog-update";
        let good = hex_encode(&Sha256::digest(body));
        let url = serve(body);
        let wrong = UpdateAsset {
            url: url.clone(),
            sha256: "deadbeef".into(),
            name: format!("../../navidog-bad-{}.dmg", std::process::id()),
            size: 0,
        };
        assert_eq!(
            download_verified(&wrong, &AtomicBool::new(false), |_, _| {}),
            Err(InstallError::Checksum)
        );
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
            size: 0,
        };
        let mut ticks = Vec::new();
        let path = download_verified(&right, &AtomicBool::new(false), |done, total| {
            ticks.push((done, total));
        })
        .unwrap();
        assert!(
            ticks.windows(2).all(|pair| pair[0].0 <= pair[1].0),
            "{ticks:?}"
        );
        assert_eq!(ticks.first().copied(), Some((0, body.len() as u64)));
        assert_eq!(
            ticks.last().copied(),
            Some((body.len() as u64, body.len() as u64))
        );
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
