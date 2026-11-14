//! Ctrl+C clipboard fallback with best-effort backup/restore.
//!
//! Safety rules implemented here:
//! - never touch a clipboard that holds files, images or private formats
//! - never send Ctrl+C while the user still holds a modifier key
//! - never overwrite content that another program wrote after our copy
//! - restore every text-ish format we backed up (plain text, HTML, RTF)

use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;

const VK_CONTROL_CODE: u32 = 0x11;
const VK_C_CODE: u32 = 0x43;

const CLIPBOARD_RETRIES: usize = 10;
const CLIPBOARD_RETRY_DELAY_MS: u64 = 20;
const MAX_TEXT_CHARS: usize = 200_000;
const MAX_BACKUP_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Default)]
struct ClipboardBackup {
    entries: Vec<(u32, Vec<u8>)>,
}

fn open_clipboard() -> Result<(), String> {
    for _ in 0..CLIPBOARD_RETRIES {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Ok(());
        }
        sleep(Duration::from_millis(CLIPBOARD_RETRY_DELAY_MS));
    }
    Err("剪贴板被其他程序占用".into())
}

fn format_name(format: u32) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClipboardFormatNameW(format, &mut buf) };
    if len > 0 {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        format!("#{format}")
    }
}

/// Backs up every HGLOBAL-based format so the clipboard can be restored
/// byte-for-byte. Fails closed on file lists and on non-memory formats
/// (bitmaps, metafiles), which cannot be copied safely this way.
fn backup_clipboard() -> Result<ClipboardBackup, String> {
    open_clipboard()?;
    let mut backup = ClipboardBackup::default();

    unsafe {
        let mut format = EnumClipboardFormats(0);
        while format != 0 {
            if format == CF_HDROP {
                let _ = CloseClipboard();
                return Err("剪贴板里是文件，已跳过回退以保护你的复制内容".into());
            }

            let handle = match GetClipboardData(format) {
                Ok(h) => h,
                Err(_) => {
                    format = EnumClipboardFormats(format);
                    continue;
                }
            };
            let hglobal = HGLOBAL(handle.0);
            let size = GlobalSize(hglobal);
            if size == 0 {
                let name = format_name(format);
                let _ = CloseClipboard();
                return Err(format!(
                    "剪贴板包含无法备份的格式「{name}」，已跳过回退以保护原内容"
                ));
            }
            if size > MAX_BACKUP_BYTES {
                let _ = CloseClipboard();
                return Err("剪贴板内容过大，已跳过回退".into());
            }

            let ptr = GlobalLock(hglobal) as *const u8;
            if ptr.is_null() {
                let _ = CloseClipboard();
                return Err("无法读取剪贴板内容，已跳过回退".into());
            }
            let data = std::slice::from_raw_parts(ptr, size).to_vec();
            let _ = GlobalUnlock(hglobal);
            backup.entries.push((format, data));

            format = EnumClipboardFormats(format);
        }
        let _ = CloseClipboard();
    }

    Ok(backup)
}

fn restore_clipboard(backup: &ClipboardBackup) -> Result<(), String> {
    open_clipboard()?;
    unsafe {
        EmptyClipboard().map_err(|e| format!("EmptyClipboard: {e}"))?;

        for (format, data) in &backup.entries {
            let Ok(hglobal) = GlobalAlloc(GMEM_MOVEABLE, data.len()) else {
                continue;
            };
            let dst = GlobalLock(hglobal) as *mut u8;
            if dst.is_null() {
                let _ = GlobalFree(Some(hglobal));
                continue;
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
            let _ = GlobalUnlock(hglobal);

            if SetClipboardData(*format, Some(HANDLE(hglobal.0))).is_err() {
                // Ownership was not transferred on failure.
                let _ = GlobalFree(Some(hglobal));
            }
        }

        let _ = CloseClipboard();
    }
    Ok(())
}

/// Puts plain text on the clipboard (used by the result window's copy button).
pub fn set_text(text: &str) -> Result<(), String> {
    open_clipboard()?;
    unsafe {
        EmptyClipboard().map_err(|e| format!("EmptyClipboard: {e}"))?;
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * std::mem::size_of::<u16>();
        let hglobal = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| format!("GlobalAlloc: {e}"))?;
        let dst = GlobalLock(hglobal) as *mut u16;
        if dst.is_null() {
            let _ = GlobalFree(Some(hglobal));
            let _ = CloseClipboard();
            return Err("GlobalLock 失败".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        let _ = GlobalUnlock(hglobal);
        if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hglobal.0))).is_err() {
            let _ = GlobalFree(Some(hglobal));
            let _ = CloseClipboard();
            return Err("SetClipboardData 失败".into());
        }
        let _ = CloseClipboard();
        Ok(())
    }
}

fn read_clipboard_text() -> Option<String> {
    if open_clipboard().is_err() {
        return None;
    }
    unsafe {
        let text = if IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() {
            read_unicode_block().ok()
        } else {
            None
        };
        let _ = CloseClipboard();
        text
    }
}

unsafe fn read_unicode_block() -> Result<String, String> {
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT).map_err(|e| e.to_string())?;
        let hglobal = HGLOBAL(handle.0);
        let ptr = GlobalLock(hglobal) as *const u16;
        if ptr.is_null() {
            return Err("GlobalLock 失败".into());
        }
        let mut len = 0usize;
        while len < MAX_TEXT_CHARS {
            if *ptr.add(len) == 0 {
                break;
            }
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        let text = String::from_utf16_lossy(slice);
        let _ = GlobalUnlock(hglobal);
        Ok(text)
    }
}

fn modifiers_down() -> bool {
    const VKS: [i32; 5] = [
        0x11, // Ctrl
        0x12, // Alt
        0x10, // Shift
        0x5B, // LWin
        0x5C, // RWin
    ];
    for vk in VKS {
        let state = unsafe { GetAsyncKeyState(vk) };
        if (state as u16 & 0x8000) != 0 {
            return true;
        }
    }
    false
}

fn wait_modifiers_released(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !modifiers_down() {
            return true;
        }
        sleep(Duration::from_millis(15));
    }
    !modifiers_down()
}

fn send_ctrl_c() -> Result<(), String> {
    unsafe {
        let mut inputs: [INPUT; 4] = std::mem::zeroed();
        let seq: [(u32, bool); 4] = [
            (VK_CONTROL_CODE, false),
            (VK_C_CODE, false),
            (VK_C_CODE, true),
            (VK_CONTROL_CODE, true),
        ];
        for (i, (vk, up)) in seq.iter().enumerate() {
            inputs[i].r#type = INPUT_KEYBOARD;
            inputs[i].Anonymous = INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(*vk as u16),
                    wScan: MapVirtualKeyW(*vk, MAPVK_VK_TO_VSC) as u16,
                    dwFlags: if *up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            };
        }
        let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        if sent != inputs.len() as u32 {
            return Err("无法向目标窗口发送 Ctrl+C（可能权限不足）".into());
        }
    }
    Ok(())
}

/// Captures the selection by simulating Ctrl+C and reading the clipboard.
/// `source_hwnd` is the window that must still be foreground.
pub fn capture_via_ctrl_c(source_hwnd: isize) -> Result<String, String> {
    let hwnd = HWND(source_hwnd as *mut core::ffi::c_void);

    if unsafe { IsClipboardFormatAvailable(CF_HDROP) }.is_ok() {
        return Err("剪贴板里是文件，已跳过回退以保护你的复制内容".into());
    }

    let backup = backup_clipboard()?;
    let seq_before = unsafe { GetClipboardSequenceNumber() };

    if !wait_modifiers_released(Duration::from_millis(500)) {
        return Err("请先松开 Ctrl/Alt/Shift 再试".into());
    }
    if unsafe { GetForegroundWindow() } != hwnd {
        return Err("目标窗口已失去焦点".into());
    }

    send_ctrl_c()?;

    let deadline = Instant::now() + Duration::from_millis(900);
    let mut seq_after = seq_before;
    while Instant::now() < deadline {
        seq_after = unsafe { GetClipboardSequenceNumber() };
        if seq_after != seq_before {
            break;
        }
        sleep(Duration::from_millis(10));
    }
    if seq_after == seq_before {
        return Err("目标程序没有响应 Ctrl+C".into());
    }

    // Let the source app finish writing all clipboard formats.
    sleep(Duration::from_millis(40));
    let text = read_clipboard_text()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    // Restore only if nobody else changed the clipboard since our copy.
    sleep(Duration::from_millis(80));
    if unsafe { GetClipboardSequenceNumber() } == seq_after {
        let _ = restore_clipboard(&backup);
    }

    text.ok_or_else(|| "没有从剪贴板读到文本".into())
}
