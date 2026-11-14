use serde::{Deserialize, Serialize};

/// Current official DeepSeek model name (see Models & Pricing docs).
pub const DEFAULT_MODEL: &str = "deepseek-flash";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub base_url: String,
    pub model: String,
    pub auto_popup: bool,
    pub translate_target: String,
    /// When UIA cannot read the selection, the auto-popup path may simulate
    /// Ctrl+C and restore the clipboard afterwards.
    pub clipboard_fallback_auto: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.deepseek.com".to_string(),
            model: DEFAULT_MODEL.to_string(),
            auto_popup: true,
            translate_target: "中文".to_string(),
            clipboard_fallback_auto: true,
        }
    }
}

impl AppSettings {
    /// Drops model names that the official API no longer accepts.
    pub fn migrate_model(&mut self) {
        let model = self.model.trim();
        if model.is_empty()
            || matches!(
                model,
                "deepseek-chat" | "deepseek-reasoner" | "deepseek-v3" | "deepseek-v3.1"
            )
        {
            self.model = DEFAULT_MODEL.to_string();
        }
    }

    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Only HTTPS is accepted; plain HTTP is allowed for a local test server.
    pub fn endpoint_is_allowed(&self) -> bool {
        let url = self.endpoint();
        url.starts_with("https://")
            || url.starts_with("http://127.0.0.1")
            || url.starts_with("http://localhost")
    }
}
