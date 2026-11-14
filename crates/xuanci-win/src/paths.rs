//! Application data paths.

use std::path::PathBuf;

pub fn app_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("XuanCi")
}

pub fn ensure_app_data_dir() -> Result<PathBuf, String> {
    let dir = app_data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建数据目录失败：{e}"))?;
    Ok(dir)
}

pub fn settings_path() -> PathBuf {
    app_data_dir().join("settings.json")
}

pub fn secrets_path() -> PathBuf {
    app_data_dir().join("secrets.bin")
}
