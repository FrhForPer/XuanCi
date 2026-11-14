//! Thin helpers around the Win32 API.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

pub const HOTKEY_TRANSLATE_ID: i32 = 0x1001;
pub const HOTKEY_ASK_AI_ID: i32 = 0x1002;

/// Link probe: proves user32 is wired up correctly.
pub fn foreground_window() -> HWND {
    unsafe { GetForegroundWindow() }
}

pub fn hwnd_is_valid(hwnd: HWND) -> bool {
    !hwnd.0.is_null()
}
