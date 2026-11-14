//! UI Automation based selection capture.
//!
//! All calls are blocking and can hang on third-party providers, so callers
//! must invoke this from a dedicated background thread and enforce a timeout.

use windows::core::Interface;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationTextPattern,
    IUIAutomationTextPattern2, UIA_TextPattern2Id, UIA_TextPatternId,
};

pub const MAX_CAPTURE_CHARS: i32 = 20_000;

#[derive(Debug, Clone)]
pub struct UiaCapture {
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub enum CaptureError {
    PasswordField,
    EmptySelection,
    Uia(String),
    ComInit(String),
    Timeout,
    CircuitOpen,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::PasswordField => write!(f, "密码框不可读取"),
            CaptureError::EmptySelection => write!(f, "没有检测到选中的文本"),
            CaptureError::Uia(e) => write!(f, "UI Automation 取词失败：{e}"),
            CaptureError::ComInit(e) => write!(f, "COM 初始化失败：{e}"),
            CaptureError::Timeout => write!(f, "UI Automation 响应超时，已临时熔断"),
            CaptureError::CircuitOpen => write!(f, "UI Automation 正处于熔断期，请稍后再试"),
        }
    }
}

pub type CaptureResult = Result<UiaCapture, CaptureError>;

/// Runs UIA on a throwaway worker thread and gives up after `timeout`.
/// A hung provider leaks its thread; the circuit breaker prevents piling up.
pub fn capture_with_timeout(timeout: std::time::Duration) -> CaptureResult {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::sync::OnceLock;
    use std::time::Instant;

    static START: OnceLock<Instant> = OnceLock::new();
    static BLOCKED_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

    let now_ms = || START.get_or_init(Instant::now).elapsed().as_millis() as u64;

    if now_ms() < BLOCKED_UNTIL_MS.load(Ordering::Relaxed) {
        return Err(CaptureError::CircuitOpen);
    }

    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let _ = tx.send(capture_selection_blocking());
        });
    if spawned.is_err() {
        return Err(CaptureError::Uia("无法创建 UIA 线程".into()));
    }

    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => {
            BLOCKED_UNTIL_MS.store(now_ms() + 45_000, Ordering::Relaxed);
            Err(CaptureError::Timeout)
        }
        Err(RecvTimeoutError::Disconnected) => {
            Err(CaptureError::Uia("UIA 线程异常退出".into()))
        }
    }
}

/// Captures the current selection. Call this on a worker thread only.
pub fn capture_selection_blocking() -> CaptureResult {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        // RPC_E_CHANGED_MODE means this thread already uses another apartment;
        // that is fatal for our purposes because UIA needs MTA here.
        if hr.is_err() {
            return Err(CaptureError::ComInit(format!("{:?}", hr)));
        }
        let result = capture_inner();
        CoUninitialize();
        result
    }
}

unsafe fn capture_inner() -> CaptureResult {
    unsafe {
    let automation: IUIAutomation =
        CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| CaptureError::Uia(format!("CoCreateInstance: {e}")))?;

    if let Ok(a2) = automation.cast::<IUIAutomation2>() {
        let _ = a2.SetConnectionTimeout(1000);
        let _ = a2.SetTransactionTimeout(2000);
    }

    let focused = automation
        .GetFocusedElement()
        .map_err(|e| CaptureError::Uia(format!("GetFocusedElement: {e}")))?;

    if let Ok(is_password) = focused.CurrentIsPassword() {
        if is_password.as_bool() {
            return Err(CaptureError::PasswordField);
        }
    }

    let control_type = focused.CurrentControlType().map(|t| t.0).unwrap_or(-1);
    let control_name = focused
        .CurrentName()
        .map(|name| name.to_string())
        .unwrap_or_default();

    let pattern: IUIAutomationTextPattern = focused
        .GetCurrentPatternAs(UIA_TextPatternId)
        .or_else(|_| {
            let p2: IUIAutomationTextPattern2 =
                focused.GetCurrentPatternAs(UIA_TextPattern2Id)?;
            p2.cast::<IUIAutomationTextPattern>()
        })
        .map_err(|e| {
            CaptureError::Uia(format!(
                "TextPattern: {e} (control_type={control_type}, name={control_name:?})"
            ))
        })?;

    let ranges = pattern
        .GetSelection()
        .map_err(|e| CaptureError::Uia(format!("GetSelection: {e}")))?;
    let count = ranges
        .Length()
        .map_err(|e| CaptureError::Uia(format!("Length: {e}")))?;

    let mut out = String::new();
    let mut truncated = false;

    for i in 0..count {
        let remaining = MAX_CAPTURE_CHARS - out.chars().count() as i32;
        if remaining <= 0 {
            truncated = true;
            break;
        }
        let range = ranges
            .GetElement(i)
            .map_err(|e| CaptureError::Uia(format!("GetElement({i}): {e}")))?;
        let text = range
            .GetText(remaining)
            .map_err(|e| CaptureError::Uia(format!("GetText: {e}")))?;
        let piece = text.to_string();
        if piece.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&piece);
    }

    let text = out.trim().to_string();
    if text.is_empty() {
        return Err(CaptureError::EmptySelection);
    }

    Ok(UiaCapture { text, truncated })
    }
}
