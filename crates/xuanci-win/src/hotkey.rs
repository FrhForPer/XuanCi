//! Global hotkey registration.

use windows::Win32::Foundation::{HWND, WIN32_ERROR};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_NOREPEAT: u32 = 0x4000;

pub fn modifiers(flags: u32) -> HOT_KEY_MODIFIERS {
    HOT_KEY_MODIFIERS(flags)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyError {
    /// The combination is already registered by another program.
    AlreadyRegistered,
    Other(u32),
}

pub fn register(hwnd: HWND, id: i32, mods: u32, vk: u32) -> Result<(), HotkeyError> {
    unsafe {
        RegisterHotKey(Some(hwnd), id, modifiers(mods), vk).map_err(|e| {
            let code = (e.code().0 as u32) & 0xFFFF;
            if code == WIN32_ERROR(1409).0 {
                HotkeyError::AlreadyRegistered
            } else {
                HotkeyError::Other(code)
            }
        })
    }
}

pub fn unregister(hwnd: HWND, id: i32) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), id);
    }
}
