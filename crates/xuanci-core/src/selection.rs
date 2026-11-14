use serde::{Deserialize, Serialize};

/// How the selected text was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureMethod {
    Uia,
    Clipboard,
    Cached,
}

/// A snapshot of the user's current text selection plus the source window.
#[derive(Debug, Clone)]
pub struct SelectionContext {
    pub text: String,
    pub source_process_name: Option<String>,
    pub source_window_title: Option<String>,
    pub source_hwnd: isize,
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub capture_method: CaptureMethod,
    pub is_truncated: bool,
}

impl SelectionContext {
    pub fn len_chars(&self) -> usize {
        self.text.chars().count()
    }
}
