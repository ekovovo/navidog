//! In-app update check and install.
//!
//! Stable builds read `latest.json` from the latest GitHub release. The
//! pre-release channel (on by default for a nightly build) walks the release
//! list and also offers the rolling `nightly` tag, compared by publish time.
//! A download with a checksum is installed only when the SHA-256 matches.
//! The nightly asset list has no checksum, so that download is installed
//! without one.

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
use rust_i18n::t;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const MANIFEST_URL: &str = "https://github.com/xhofe/navidog/releases/latest/download/latest.json";
const LATEST_API: &str = "https://api.github.com/repos/xhofe/navidog/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/xhofe/navidog/releases/latest";
const RELEASE_LIST_API: &str = "https://api.github.com/repos/xhofe/navidog/releases?per_page=15";
const NIGHTLY_TAG: &str = "nightly";
const NIGHTLY_GRACE: chrono::TimeDelta = chrono::TimeDelta::minutes(15);
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

pub enum Delivery {
    /// The running `.app` was replaced. Restart to use it.
    Replaced,
    /// The installer was handed to the operating system.
    HandedToOs,
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
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// `Some(version)` when `raw` is a semver strictly newer than `current`.
pub fn newer_version(raw: &str, current: &str) -> Option<String> {
    let latest = Version::parse(raw.trim().trim_start_matches('v')).ok()?;
    let current = Version::parse(current).ok()?;
    (latest > current).then(|| latest.to_string())
}

/// A nightly published at `published_at` is newer than this build when it is
/// more than 15 minutes later. The grace skips the upload that is still
/// finishing for the build that is running.
pub fn nightly_is_newer(published_at: &str, built_at: &str) -> bool {
    let (Ok(published), Ok(built)) = (
        chrono::DateTime::parse_from_rfc3339(published_at.trim()),
        chrono::DateTime::parse_from_rfc3339(built_at.trim()),
    ) else {
        return false;
    };
    published - built > NIGHTLY_GRACE
}

/// `(os, arch, kind)` of a file the publish workflow uploads.
pub fn asset_identity(name: &str) -> Option<(&'static str, String, &'static str)> {
    if let Some(arch) = name
        .strip_prefix("Navidog-")
        .and_then(|rest| rest.strip_suffix(".dmg"))
    {
        return Some(("macos", arch.to_string(), "dmg"));
    }
    let rest = name.strip_prefix("navidog-")?;
    if let Some(arch) = rest.strip_suffix(".AppImage.tar.gz") {
        return Some(("linux", arch.to_string(), "appimage"));
    }
    let (os, file) = rest.split_once('-')?;
    let os = match os {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        _ => return None,
    };
    const KINDS: [(&str, &str); 6] = [
        (".tar.gz", "tarball"),
        (".msi.zip", "msizip"),
        (".msi", "msi"),
        (".zip", "zip"),
        (".deb", "deb"),
        (".rpm", "rpm"),
    ];
    KINDS.iter().find_map(|(suffix, kind)| {
        file.strip_suffix(suffix)
            .map(|arch| (os, arch.to_string(), *kind))
    })
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

fn assets_from_api(assets: &[GithubAsset]) -> Vec<ManifestAsset> {
    assets
        .iter()
        .filter_map(|asset| {
            let (os, arch, kind) = asset_identity(&asset.name)?;
            Some(ManifestAsset {
                os: os.to_string(),
                arch,
                kind: kind.to_string(),
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
                sha256: String::new(),
            })
        })
        .collect()
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

/// Blocking. Run it off the UI thread.
pub fn fetch_latest_release(include_prerelease: bool) -> Result<Option<UpdateInfo>, String> {
    if include_prerelease {
        match fetch_from_release_list() {
            Ok(found) => return Ok(found),
            Err(e) => tracing::debug!(error = %e, "update check: release list unavailable"),
        }
    }
    match fetch_from_manifest() {
        Ok(found) => Ok(found),
        Err(e) => {
            tracing::debug!(error = %e, "update check: manifest unavailable");
            fetch_from_api()
        }
    }
}

fn fetch_from_manifest() -> Result<Option<UpdateInfo>, String> {
    let manifest: Manifest =
        serde_json::from_str(&http_get(MANIFEST_URL, Duration::from_secs(15))?)
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

fn fetch_from_api() -> Result<Option<UpdateInfo>, String> {
    let release: GithubRelease =
        serde_json::from_str(&http_get(LATEST_API, Duration::from_secs(15))?)
            .map_err(|e| e.to_string())?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let Some(latest) = newer_version(&release.tag_name, env!("CARGO_PKG_VERSION")) else {
        return Ok(None);
    };
    let page_url = if release.html_url.is_empty() {
        RELEASES_PAGE.to_string()
    } else {
        release.html_url
    };
    Ok(Some(UpdateInfo {
        version: latest,
        current: current_label(),
        page_url,
        notes: release.body.trim().to_string(),
        asset: None,
    }))
}

fn fetch_from_release_list() -> Result<Option<UpdateInfo>, String> {
    let releases: Vec<GithubRelease> =
        serde_json::from_str(&http_get(RELEASE_LIST_API, Duration::from_secs(15))?)
            .map_err(|e| e.to_string())?;
    for release in releases.iter().filter(|r| !r.draft) {
        let page_url = if release.html_url.trim().is_empty() {
            RELEASES_PAGE.to_string()
        } else {
            release.html_url.clone()
        };
        if release.tag_name == NIGHTLY_TAG {
            if !nightly_is_newer(&release.published_at, BUILD_TIMESTAMP) {
                continue;
            }
            let published = release.published_at.get(..10).unwrap_or(NIGHTLY_TAG);
            return Ok(Some(UpdateInfo {
                version: format!("{NIGHTLY_TAG} {published}"),
                current: current_label(),
                page_url,
                notes: release.body.trim().to_string(),
                asset: pick_asset_for(
                    &assets_from_api(&release.assets),
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                ),
            }));
        }
        let Some(latest) = newer_version(&release.tag_name, env!("CARGO_PKG_VERSION")) else {
            continue;
        };
        let manifest_url = format!(
            "https://github.com/xhofe/navidog/releases/download/{}/latest.json",
            release.tag_name
        );
        let asset = http_get(&manifest_url, Duration::from_secs(15))
            .ok()
            .and_then(|text| serde_json::from_str::<Manifest>(&text).ok())
            .and_then(|manifest| {
                pick_asset_for(
                    &manifest.assets,
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                )
            })
            .or_else(|| {
                pick_asset_for(
                    &assets_from_api(&release.assets),
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                )
            });
        return Ok(Some(UpdateInfo {
            version: latest,
            current: current_label(),
            page_url,
            notes: release.body.trim().to_string(),
            asset,
        }));
    }
    Ok(None)
}

fn http_get(url: &str, timeout: Duration) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
}

/// Downloads `asset` and checks its SHA-256 when the manifest has one.
pub fn download_verified(asset: &UpdateAsset) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("navidog-update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&asset.name);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into();
    let mut response = agent
        .get(&asset.url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| e.to_string())?;
    let mut reader = response.body_mut().as_reader();
    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_DOWNLOAD {
            let _ = std::fs::remove_file(&path);
            return Err("download is larger than 512 MB".into());
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    let got = hex_encode(&hasher.finalize());
    if !asset.sha256.is_empty() && !got.eq_ignore_ascii_case(asset.sha256.trim()) {
        let _ = std::fs::remove_file(&path);
        return Err("checksum".into());
    }
    Ok(path)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

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
    let aside = std::env::temp_dir().join(format!("navidog-old-{}.app", std::process::id()));
    let _ = std::fs::remove_dir_all(&aside);
    std::fs::rename(app, &aside).map_err(|e| e.to_string())?;
    let copied = std::process::Command::new("ditto")
        .arg(&src)
        .arg(app)
        .status();
    let _ = std::process::Command::new("hdiutil")
        .args(["detach"])
        .arg(&mount)
        .status();
    match copied {
        Ok(status) if status.success() => Ok(()),
        _ => {
            let _ = std::fs::rename(&aside, app);
            Err("could not copy the new app into place".into())
        }
    }
}

#[cfg(target_os = "macos")]
pub fn relaunch() {
    if let Ok(exe) = std::env::current_exe()
        && let Some(app) = bundle_from_exe(&exe)
    {
        let _ = std::process::Command::new("open").arg(app).spawn();
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

/// Menu item: always checks, and reports "up to date" and errors.
pub fn manual_check(window: &mut Window, cx: &mut App) {
    begin_check(true, window, cx);
}

fn begin_check(manual: bool, _window: &mut Window, cx: &mut App) {
    let include = Settings::global(cx).include_prerelease(is_nightly_build());
    Settings::update(cx, |s| s.update_checked_unix = Some(unix_now()));
    cx.spawn(async move |cx| {
        let found = cx
            .background_spawn(async move { fetch_latest_release(include) })
            .await;
        cx.update(|cx| present(found, manual, cx));
    })
    .detach();
}

fn present(found: Result<Option<UpdateInfo>, String>, manual: bool, cx: &mut App) {
    let Some(handle) = cx.active_window() else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| match found {
        Ok(Some(info)) => open_dialog(info, window, cx),
        Ok(None) if manual => {
            window.push_notification(t!("update.uptodate").to_string(), cx);
        }
        Err(detail) if manual => {
            window.push_notification(t!("update.failed", detail = detail).to_string(), cx);
        }
        _ => {}
    });
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
                    download_verified(&asset).and_then(|path| install(&path).map(|d| (d, path)))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((Delivery::Replaced, _)) => {
                        window.close_dialog(cx);
                        open_restart(window, cx);
                    }
                    Ok((Delivery::HandedToOs, _)) => {
                        window.push_notification(t!("update.handed_off").to_string(), cx);
                        window.close_dialog(cx);
                    }
                    Err(detail) if detail == "checksum" => {
                        window.push_notification(t!("update.bad_checksum").to_string(), cx);
                        cx.open_url(&page);
                    }
                    Err(detail) => {
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

fn notes_preview(notes: &str) -> String {
    let notes = notes.trim();
    if notes.chars().count() <= 800 {
        return notes.to_string();
    }
    let mut end = 800;
    while !notes.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &notes[..end])
}

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
    fn newer_version_compares_semver() {
        assert_eq!(newer_version("v0.2.0", "0.1.0").as_deref(), Some("0.2.0"));
        assert!(newer_version("0.1.0", "0.1.0").is_none());
        assert!(newer_version("0.0.9", "0.1.0").is_none());
        assert!(newer_version("nightly", "0.1.0").is_none());
    }

    #[test]
    fn nightly_is_newer_after_the_grace_window() {
        assert!(!nightly_is_newer(
            "2026-10-16T00:10:00Z",
            "2026-10-16T00:00:00Z"
        ));
        assert!(nightly_is_newer(
            "2026-10-16T00:20:00Z",
            "2026-10-16T00:00:00Z"
        ));
        assert!(!nightly_is_newer("not-a-date", "2026-10-16T00:00:00Z"));
    }

    #[test]
    fn asset_names_map_to_os_arch_and_kind() {
        assert_eq!(
            asset_identity("Navidog-aarch64.dmg"),
            Some(("macos", "aarch64".into(), "dmg"))
        );
        assert_eq!(
            asset_identity("navidog-aarch64.AppImage.tar.gz"),
            Some(("linux", "aarch64".into(), "appimage"))
        );
        assert_eq!(
            asset_identity("navidog-windows-x86_64.msi"),
            Some(("windows", "x86_64".into(), "msi"))
        );
        assert_eq!(
            asset_identity("navidog-windows-x86_64.msi.zip"),
            Some(("windows", "x86_64".into(), "msizip"))
        );
        assert_eq!(
            asset_identity("navidog-linux-x86_64.tar.gz"),
            Some(("linux", "x86_64".into(), "tarball"))
        );
        assert!(asset_identity("SHA256SUMS").is_none());
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
    fn a_running_app_bundle_is_the_nearest_app_ancestor() {
        let path = Path::new("/Applications/Navidog.app/Contents/MacOS/navidog");
        assert_eq!(
            bundle_from_exe(path).unwrap(),
            PathBuf::from("/Applications/Navidog.app")
        );
        assert!(bundle_from_exe(Path::new("/usr/local/bin/navidog")).is_none());
    }
}
