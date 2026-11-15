//! Local settings and DPAPI-protected API key.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use xuanci_core::settings::AppSettings;
use xuanci_core::templates::{default_templates, PromptTemplate};
use xuanci_win::{crypto, paths};

pub fn load_settings() -> AppSettings {
    let mut settings: AppSettings = fs::read_to_string(paths::settings_path())
        .ok()
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default();

    let previous = serde_json::to_string(&settings).unwrap_or_default();
    settings.normalize();
    let current = serde_json::to_string(&settings).unwrap_or_default();
    if previous != current {
        // Persist migrations (legacy model names, missing provider list).
        let _ = save_settings(&settings);
    }
    settings
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    paths::ensure_app_data_dir()?;
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("序列化设置失败：{e}"))?;
    fs::write(paths::settings_path(), text).map_err(|e| format!("写入设置失败：{e}"))
}

/// Loads all per-provider keys. Migrates the legacy single-key file.
pub fn load_api_keys(settings: &AppSettings) -> HashMap<String, String> {
    let Ok(cipher) = fs::read(paths::secrets_path()) else {
        return HashMap::new();
    };
    let Ok(plain) = crypto::unprotect(&cipher) else {
        return HashMap::new();
    };
    let Ok(text) = String::from_utf8(plain) else {
        return HashMap::new();
    };

    if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&text) {
        return map;
    }

    // Legacy: the file contained the raw key for the old single provider.
    let legacy_key = text.trim();
    if legacy_key.is_empty() {
        return HashMap::new();
    }
    let mut map = HashMap::new();
    map.insert(settings.active_provider.clone(), legacy_key.to_string());
    let _ = save_api_keys(&map);
    map
}

pub fn save_api_keys(keys: &HashMap<String, String>) -> Result<(), String> {
    paths::ensure_app_data_dir()?;
    let text = serde_json::to_string(keys).map_err(|e| format!("序列化密钥失败：{e}"))?;
    let cipher = crypto::protect(text.as_bytes())?;
    fs::write(paths::secrets_path(), cipher).map_err(|e| format!("写入密钥失败：{e}"))
}

pub fn templates_path() -> PathBuf {
    paths::app_data_dir().join("templates.json")
}

/// Loads templates.json, creating it with sane defaults on first run.
pub fn load_templates() -> Vec<PromptTemplate> {
    let path = templates_path();
    if !path.exists() {
        let defaults = default_templates();
        let _ = paths::ensure_app_data_dir();
        if let Ok(text) = serde_json::to_string_pretty(&defaults) {
            let _ = fs::write(&path, text);
        }
        return defaults;
    }

    let Ok(text) = fs::read_to_string(&path) else {
        return default_templates();
    };
    match serde_json::from_str::<Vec<PromptTemplate>>(text.trim_start_matches('\u{feff}')) {
        Ok(list) => list.into_iter().filter(|item| item.is_valid()).collect(),
        Err(_) => default_templates(),
    }
}
