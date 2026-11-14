use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    /// Some OpenAI-compatible providers reject unknown fields, so this is only
    /// set for DeepSeek.
    pub disable_thinking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatDelta {
    Content(String),
    Thinking(String),
    Done,
    Error(String),
}
