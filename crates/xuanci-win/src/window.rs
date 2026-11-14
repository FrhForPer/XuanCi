//! Hidden message-only-ish window used as the application's message sink.

use std::mem::zeroed;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{GetLastError, HINSTANCE, HWND};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, WINDOW_EX_STYLE, WNDCLASSW, WNDPROC,
    WS_OVERLAPPED,
};

pub struct HiddenWindow {
    pub hwnd: HWND,
}

/// Registers a window class and creates a hidden top-level window.
pub fn create(class_name: &str, title: &str, wndproc: WNDPROC) -> Result<HiddenWindow, String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
        let hinstance = HINSTANCE(instance.0);
        let class = HSTRING::from(class_name);
        let class_ptr = PCWSTR(class.as_ptr());

        let mut wc: WNDCLASSW = zeroed();
        wc.style = Default::default();
        wc.lpfnWndProc = wndproc;
        wc.cbClsExtra = 0;
        wc.cbWndExtra = 0;
        wc.hInstance = hinstance;
        wc.hIcon = Default::default();
        wc.hCursor = Default::default();
        wc.hbrBackground = Default::default();
        wc.lpszMenuName = PCWSTR::null();
        wc.lpszClassName = class_ptr;

        let atom = RegisterClassW(&wc);
        if atom == 0 {
            let err = GetLastError();
            // 1410 = ERROR_CLASS_ALREADY_EXISTS, harmless on restart in-process.
            if err.0 != 1410 {
                return Err(format!("RegisterClassW failed: {}", err.0));
            }
        }

        let title_w = HSTRING::from(title);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_ptr,
            PCWSTR(title_w.as_ptr()),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )
        .map_err(|e| format!("CreateWindowExW: {e}"))?;

        Ok(HiddenWindow { hwnd })
    }
}

pub fn default_proc(hwnd: HWND, msg: u32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
