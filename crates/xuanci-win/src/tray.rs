//! Tray icon via Shell_NotifyIconW.

use std::mem::zeroed;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    LoadIconW, HICON, IDI_APPLICATION, WM_APP,
};

pub const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 1;
pub const TRAY_ID: u32 = 1;

fn fill_tip(nid: &mut NOTIFYICONDATAW, tip: &str) {
    let wide: Vec<u16> = tip.encode_utf16().take(nid.szTip.len() - 1).collect();
    nid.szTip[..wide.len()].copy_from_slice(&wide);
}

pub fn add(hwnd: HWND, tip: &str) -> Result<(), String> {
    unsafe {
        let icon: HICON = LoadIconW(None, IDI_APPLICATION)
            .map_err(|e| format!("LoadIconW: {e}"))?;

        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = TRAY_CALLBACK_MESSAGE;
        nid.hIcon = icon;
        fill_tip(&mut nid, tip);

        Shell_NotifyIconW(NIM_ADD, &nid)
            .ok()
            .map_err(|e| format!("Shell_NotifyIconW(NIM_ADD): {e}"))
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_ID;
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// Returns true when the tray callback wrapped a right-button-up event.
pub fn is_right_click(lparam: isize) -> bool {
    const WM_RBUTTONUP: u32 = 0x0205;
    (lparam as u32 & 0xFFFF) == WM_RBUTTONUP
}

pub fn icon_handle() -> Option<HICON> {
    unsafe { LoadIconW(None, IDI_APPLICATION).ok() }
}

/// Unused but keeps the PCWSTR import meaningful for future custom icons.
#[allow(dead_code)]
fn _pcwstr_marker() -> PCWSTR {
    PCWSTR::null()
}
