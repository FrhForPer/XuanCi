//! Private window messages shared between platform helpers and the app.

use windows::Win32::UI::WindowsAndMessaging::WM_APP;

pub const WM_XUANCI_MOUSE: u32 = WM_APP + 10;
pub const WM_XUANCI_FOREGROUND: u32 = WM_APP + 11;
pub const WM_XUANCI_ACTION: u32 = WM_APP + 12;
pub const WM_XUANCI_CAPTURED: u32 = WM_APP + 13;
pub const WM_XUANCI_CAPTURE_FAILED: u32 = WM_APP + 14;
pub const WM_XUANCI_RESULT_CHUNK: u32 = WM_APP + 20;
pub const WM_XUANCI_RESULT_DONE: u32 = WM_APP + 21;

pub const MOUSE_DOWN: usize = 1;
pub const MOUSE_UP: usize = 2;
pub const MOUSE_WHEEL: usize = 4;
pub const MOUSE_OTHER_BUTTON: usize = 5;

pub fn pack_point(x: i32, y: i32) -> isize {
    ((x as u32 as usize) & 0xFFFF_FFFF) as isize
        | ((((y as u32 as usize) & 0xFFFF_FFFF) << 32) as isize)
}

pub fn unpack_point(packed: isize) -> (i32, i32) {
    let value = packed as usize;
    let x = (value & 0xFFFF_FFFF) as u32 as i32;
    let y = ((value >> 32) & 0xFFFF_FFFF) as u32 as i32;
    (x, y)
}
