use serde::{Deserialize, Serialize};

/// Current official DeepSeek model name (see Models & Pricing docs).
pub const DEFAULT_MODEL: &str = "deepseek-flash";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
}

fn default_base_url() -> String {
    "https://api.deepseek.com".to_string()
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

fn default_auto_popup() -> bool {
    true
}

fn default_translate_target() -> String {
    "中文".to_string()
}

fn default_clipboard_fallback_auto() -> bool {
    true
}

fn default_markdown_render() -> bool {
    true
}

fn default_latex_plugin_enabled() -> bool {
    true
}

/// Missing `providers` must stay empty so `normalize()` can migrate the legacy
/// top-level `base_url` / `model` instead of silently falling back to DeepSeek.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Mirrors the active provider so existing request code stays simple.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_auto_popup")]
    pub auto_popup: bool,
    #[serde(default = "default_translate_target")]
    pub translate_target: String,
    /// When UIA cannot read the selection, the auto-popup path may simulate
    /// Ctrl+C and restore the clipboard afterwards.
    #[serde(default = "default_clipboard_fallback_auto")]
    pub clipboard_fallback_auto: bool,
    /// Render result Markdown into rich text. Plain text remains as fallback.
    #[serde(default = "default_markdown_render")]
    pub markdown_render: bool,
    /// Use the optional external LaTeX plugin when it is installed.
    #[serde(default = "default_latex_plugin_enabled")]
    pub latex_plugin_enabled: bool,
    #[serde(default)]
    pub providers: Vec<ProviderProfile>,
    #[serde(default)]
    pub active_provider: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            model: default_model(),
            auto_popup: default_auto_popup(),
            translate_target: default_translate_target(),
            clipboard_fallback_auto: default_clipboard_fallback_auto(),
            markdown_render: default_markdown_render(),
            latex_plugin_enabled: default_latex_plugin_enabled(),
            providers: vec![ProviderProfile {
                id: "deepseek".to_string(),
                name: "DeepSeek".to_string(),
                base_url: default_base_url(),
                model: default_model(),
            }],
            active_provider: "deepseek".to_string(),
        }
    }
}

fn is_legacy_deepseek_model(model: &str) -> bool {
    matches!(
        model.trim(),
        "deepseek-chat" | "deepseek-reasoner" | "deepseek-v3" | "deepseek-v3.1"
    )
}

/// Recognizes common provides from their Base URL.
/// Returns (stable id, display name, suggested model).
pub fn detect_provider(base_url: &str) -> (&'static str, &'static str, &'static str) {
    let url = base_url.to_ascii_lowercase();
    if url.contains("deepseek") {
        ("deepseek", "DeepSeek", "deepseek-flash")
    } else if url.contains("api.openai.com") {
        ("openai", "OpenAI", "gpt-4o-mini")
    } else if url.contains("siliconflow") {
        ("siliconflow", "硅基流动", "Qwen/Qwen2.5-7B-Instruct")
    } else if url.contains("dashscope") {
        ("dashscope", "通义千问", "qwen-plus")
    } else if url.contains("bigmodel") {
        ("zhipu", "智谱 GLM", "glm-4-flash")
    } else if url.contains("moonshot") {
        ("moonshot", "Moonshot", "moonshot-v1-8k")
    } else if url.contains("11434") {
        ("ollama", "Ollama (本地)", "qwen2.5:7b")
    } else if url.contains("1234") {
        ("lmstudio", "LM Studio (本地)", "")
    } else {
        ("custom", "自定义", "")
    }
}

impl AppSettings {
    /// Normalizes old settings files and keeps the active provider consistent.
    pub fn normalize(&mut self) {
        if self.providers.is_empty() {
            let (id, name, default_model) = detect_provider(&self.base_url);
            let model = if self.model.trim().is_empty() {
                default_model.to_string()
            } else {
                self.model.clone()
            };
            self.providers.push(ProviderProfile {
                id: id.to_string(),
                name: name.to_string(),
                base_url: self.base_url.clone(),
                model,
            });
            self.active_provider = id.to_string();
        }

        for provider in &mut self.providers {
            if provider.model.trim().is_empty() {
                provider.model = detect_provider(&provider.base_url).2.to_string();
            }
            if is_legacy_deepseek_model(&provider.model) {
                provider.model = DEFAULT_MODEL.to_string();
            }
        }

        if self.active_provider.is_empty()
            || !self
                .providers
                .iter()
                .any(|provider| provider.id == self.active_provider)
        {
            self.active_provider = self.providers[0].id.clone();
        }

        // Repair a target language corrupted by an earlier ANSI/UTF-8 mixup.
        if self.translate_target.trim().is_empty() || self.translate_target == "涓枃" {
            self.translate_target = "中文".to_string();
        }

        self.sync_active();
        if is_legacy_deepseek_model(&self.model) {
            self.model = DEFAULT_MODEL.to_string();
        }
    }

    pub fn sync_active(&mut self) {
        if let Some(provider) = self
            .providers
            .iter()
            .find(|provider| provider.id == self.active_provider)
        {
            self.base_url = provider.base_url.clone();
            self.model = provider.model.clone();
        }
    }

    pub fn active(&self) -> Option<&ProviderProfile> {
        self.providers
            .iter()
            .find(|provider| provider.id == self.active_provider)
    }

    pub fn active_name(&self) -> String {
        self.active()
            .map(|provider| provider.name.clone())
            .unwrap_or_else(|| "未配置".to_string())
    }

    pub fn switch_provider(&mut self, id: &str) -> bool {
        if self.providers.iter().any(|provider| provider.id == id) {
            self.active_provider = id.to_string();
            self.sync_active();
            true
        } else {
            false
        }
    }

    pub fn upsert_provider(&mut self, profile: ProviderProfile) {
        if let Some(existing) = self
            .providers
            .iter_mut()
            .find(|provider| provider.id == profile.id)
        {
            *existing = profile;
        } else {
            self.providers.push(profile);
        }
        self.sync_active();
    }

    pub fn remove_provider(&mut self, id: &str) -> bool {
        if self.providers.len() <= 1 {
            return false;
        }
        let before = self.providers.len();
        self.providers.retain(|provider| provider.id != id);
        if self.providers.len() == before {
            return false;
        }
        if self.active_provider == id {
            self.active_provider = self.providers[0].id.clone();
        }
        self.sync_active();
        true
    }

    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    pub fn models_endpoint(&self) -> String {
        format!("{}/models", self.base_url.trim_end_matches('/'))
    }

    /// Only HTTPS is accepted; plain HTTP is allowed for a local test server.
    pub fn base_url_is_allowed(&self) -> bool {
        let url = self.base_url.trim_end_matches('/');
        url.starts_with("https://")
            || url.starts_with("http://127.0.0.1")
            || url.starts_with("http://localhost")
    }

    pub fn endpoint_is_allowed(&self) -> bool {
        self.base_url_is_allowed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_providers() {
        assert_eq!(detect_provider("https://api.deepseek.com").0, "deepseek");
        assert_eq!(detect_provider("https://api.openai.com/v1").0, "openai");
        assert_eq!(detect_provider("https://api.siliconflow.cn/v1").0, "siliconflow");
        assert_eq!(
            detect_provider("https://dashscope.aliyuncs.com/compatible-mode/v1").0,
            "dashscope"
        );
        assert_eq!(
            detect_provider("https://open.bigmodel.cn/api/paas/v4").0,
            "zhipu"
        );
        assert_eq!(detect_provider("https://api.moonshot.cn/v1").0, "moonshot");
        assert_eq!(detect_provider("http://127.0.0.1:11434/v1").0, "ollama");
        assert_eq!(detect_provider("http://127.0.0.1:1234/v1").0, "lmstudio");
        assert_eq!(detect_provider("https://example.com/v1").0, "custom");
    }

    #[test]
    fn migrates_legacy_single_provider_settings() {
        let mut settings = AppSettings {
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o-mini".into(),
            providers: Vec::new(),
            active_provider: String::new(),
            ..AppSettings::default()
        };
        settings.normalize();

        assert_eq!(settings.providers.len(), 1);
        assert_eq!(settings.active_provider, "openai");
        assert_eq!(settings.active().unwrap().model, "gpt-4o-mini");
        assert_eq!(settings.endpoint(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn deserializes_legacy_json_without_providers() {
        let json = r#"{
            "base_url": "https://api.openai.com/v1",
            "model": "gpt-4o-mini",
            "auto_popup": true,
            "translate_target": "中文"
        }"#;
        let mut settings: AppSettings = serde_json::from_str(json).unwrap();
        assert!(settings.providers.is_empty());
        assert!(settings.clipboard_fallback_auto);
        assert!(settings.markdown_render);
        assert!(settings.latex_plugin_enabled);

        settings.normalize();
        assert_eq!(settings.providers.len(), 1);
        assert_eq!(settings.active_provider, "openai");
        assert_eq!(settings.base_url, "https://api.openai.com/v1");
        assert_eq!(settings.model, "gpt-4o-mini");
    }

    #[test]
    fn migrates_legacy_deepseek_model_names() {
        let mut settings = AppSettings {
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-chat".into(),
            ..AppSettings::default()
        };
        settings.providers[0].model = "deepseek-chat".into();
        settings.normalize();

        assert_eq!(settings.model, DEFAULT_MODEL);
        assert_eq!(settings.providers[0].model, DEFAULT_MODEL);
    }

    #[test]
    fn switching_and_removing_providers() {
        let mut settings = AppSettings::default();
        settings.upsert_provider(ProviderProfile {
            id: "openai".into(),
            name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o-mini".into(),
        });

        assert!(settings.switch_provider("openai"));
        assert_eq!(settings.base_url, "https://api.openai.com/v1");
        assert_eq!(settings.model, "gpt-4o-mini");

        assert!(settings.remove_provider("openai"));
        assert_eq!(settings.providers.len(), 1);
        assert_eq!(settings.active_provider, "deepseek");
        assert_eq!(settings.base_url, "https://api.deepseek.com");

        assert!(!settings.remove_provider("deepseek"));
        assert_eq!(settings.providers.len(), 1);
    }

    #[test]
    fn rejects_plain_remote_http() {
        let mut settings = AppSettings::default();
        settings.base_url = "http://api.example.com/v1".into();
        assert!(!settings.base_url_is_allowed());

        settings.base_url = "http://127.0.0.1:8765".into();
        assert!(settings.base_url_is_allowed());
    }
}
