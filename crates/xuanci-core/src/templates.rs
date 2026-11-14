//! Declarative prompt-template actions (the v1 extension mechanism).

use serde::{Deserialize, Serialize};

use crate::ai::{ChatMessage, ChatRequest};
use crate::prompt::{wrap_data, DATA_RULE_PUBLIC};
use crate::settings::AppSettings;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub system_prompt: String,
    /// Must contain the `{{text}}` placeholder.
    pub user_template: String,
    #[serde(default)]
    pub temperature: Option<f32>,
}

impl PromptTemplate {
    pub fn is_valid(&self) -> bool {
        !self.id.trim().is_empty()
            && !self.name.trim().is_empty()
            && self.user_template.contains("{{text}}")
    }

    pub fn to_request(&self, settings: &AppSettings, text: &str) -> ChatRequest {
        let system = format!("{DATA_RULE_PUBLIC} {}", self.system_prompt.trim());
        let user = self.user_template.replace("{{text}}", &wrap_data(text));
        ChatRequest {
            model: settings.model.clone(),
            messages: vec![ChatMessage::system(system), ChatMessage::user(user)],
            temperature: self.temperature.unwrap_or(0.5),
            disable_thinking: false,
        }
    }
}

pub fn default_templates() -> Vec<PromptTemplate> {
    vec![
        PromptTemplate {
            id: "polish".into(),
            name: "润色".into(),
            system_prompt: "你是一名中文编辑。在不改变原意的前提下改善表达，使文字更通顺、专业、简洁。只输出润色后的文本。".into(),
            user_template: "请润色以下内容：\n{{text}}".into(),
            temperature: Some(0.4),
        },
        PromptTemplate {
            id: "summarize".into(),
            name: "总结".into(),
            system_prompt: "你是一名信息整理助手。用简洁的中文要点总结用户提供的文本，保留关键事实与数字。".into(),
            user_template: "请总结以下内容：\n{{text}}".into(),
            temperature: Some(0.3),
        },
        PromptTemplate {
            id: "explain-code".into(),
            name: "解释代码".into(),
            system_prompt: "你是一名资深工程师。解释代码的功能、关键逻辑与潜在问题，使用简洁的中文。".into(),
            user_template: "请解释以下代码：\n{{text}}".into(),
            temperature: Some(0.3),
        },
    ]
}
