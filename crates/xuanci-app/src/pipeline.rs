//! Selection capture orchestration: snapshot -> UIA -> optional clipboard fallback.

use std::time::Duration;

use xuanci_core::selection::{CaptureMethod, SelectionContext};
use xuanci_win::snapshot::{self, TargetKind};
use xuanci_win::{clipboard, uia};

const UIA_TIMEOUT: Duration = Duration::from_millis(1200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    /// User explicitly pressed a hotkey: fallback to Ctrl+C is acceptable.
    Hotkey,
    /// Automatic popup after mouse selection: UIA only, never touch the clipboard.
    AutoPopup,
}

/// Blocking. Must be called from a worker thread.
///
/// `allow_auto_fallback` controls whether the automatic mouse-popup path may
/// use the clipboard fallback when UIA cannot read the selection.
pub fn capture(mode: TriggerMode, allow_auto_fallback: bool) -> Result<SelectionContext, String> {
    let mut info =
        snapshot::foreground_target().ok_or_else(|| "无法获取前台窗口".to_string())?;

    // If one of our own windows is foreground (e.g. a result window), fall back
    // to the last external foreground window we observed.
    if info.pid == std::process::id() {
        let last = xuanci_win::foreground::last_external_foreground();
        if last != 0 {
            if let Some(external) =
                snapshot::target_for_hwnd(windows::Win32::Foundation::HWND(last as *mut _))
            {
                info = external;
            }
        }
    }

    let kind = snapshot::classify(&info);

    match kind {
        TargetKind::SelfProcess => return Err("当前窗口是 XuanCi 自身".into()),
        TargetKind::Elevated => {
            return Err("目标程序以管理员身份运行，划词助手无权读取。请手动复制后按热键。".into())
        }
        _ => {}
    }

    #[cfg(debug_assertions)]
    let force_uia_failure = std::env::var("XUANCI_DEBUG_FORCE_UIA_FAIL").is_ok();
    #[cfg(not(debug_assertions))]
    let force_uia_failure = false;

    let uia_result = if force_uia_failure {
        Err(uia::CaptureError::Uia("forced by debug env".into()))
    } else {
        uia::capture_with_timeout(UIA_TIMEOUT)
    };

    let (text, method, truncated) = match uia_result {
        Ok(cap) => (cap.text, CaptureMethod::Uia, cap.truncated),
        Err(uia_err) => {
            if matches!(&uia_err, uia::CaptureError::PasswordField) {
                return Err("密码框不可读取".into());
            }

            let fallback_allowed = snapshot::allows_clipboard_fallback(kind)
                && match mode {
                    TriggerMode::Hotkey => true,
                    TriggerMode::AutoPopup => allow_auto_fallback,
                };
            if !fallback_allowed {
                return Err(explain_failure(kind, &uia_err.to_string()));
            }

            // Give the source app a moment to finalize the selection.
            if mode == TriggerMode::AutoPopup {
                std::thread::sleep(Duration::from_millis(120));
            }

            let text = clipboard::capture_via_ctrl_c(info.hwnd)
                .map_err(|e| format!("{}；剪贴板回退失败：{e}", uia_err))?;
            (text, CaptureMethod::Clipboard, false)
        }
    };

    let (cursor_x, cursor_y) = snapshot::cursor_position();
    Ok(SelectionContext {
        text,
        source_process_name: Some(info.process_name),
        source_window_title: None,
        source_hwnd: info.hwnd,
        cursor_x,
        cursor_y,
        capture_method: method,
        is_truncated: truncated,
    })
}

fn explain_failure(kind: TargetKind, uia_error: &str) -> String {
    match kind {
        TargetKind::Console => {
            "终端窗口不支持直接取词（为避免 Ctrl+C 中断正在运行的程序）。请手动复制后粘贴到其他应用。".into()
        }
        TargetKind::IdeTerminal => {
            "IDE 的终端里无法安全模拟 Ctrl+C（会打断程序）。请在代码编辑区选中文本，或手动复制。".into()
        }
        TargetKind::Explorer => {
            "资源管理器文件列表不做剪贴板回退，以免覆盖你复制的文件。".into()
        }
        _ => format!("取词失败：{uia_error}"),
    }
}
