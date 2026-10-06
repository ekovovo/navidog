use std::path::{Path, PathBuf};

/// 配置目录。`NAVIDOG_HOME` 可覆盖（测试与多配置并存时使用）。
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NAVIDOG_HOME") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/navidog")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or(home)
            .join("navidog")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("navidog")
    }
}

pub fn log_dir() -> PathBuf {
    if cfg!(target_os = "macos") && std::env::var_os("NAVIDOG_HOME").is_none() {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        return home.join("Library/Logs/navidog");
    }
    config_dir().join("logs")
}

/// 每个连接的设置位置（M3 起存放查询文件等）。
pub fn profile_dir(connection_id: &str) -> PathBuf {
    config_dir().join("profiles").join(connection_id)
}

/// 先写临时文件再原子替换，避免崩溃时损坏配置。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
