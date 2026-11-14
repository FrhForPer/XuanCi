//! Low-level mouse hook with drag detection.
//!
//! The hook callback stays tiny: it tracks drag distance and posts a few
//! private messages. All UI work happens on the main thread.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, MSG, MSLLHOOKSTRUCT, PostMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
};

use crate::messages::{
    pack_point, MOUSE_DOWN, MOUSE_OTHER_BUTTON, MOUSE_UP, MOUSE_WHEEL, WM_XUANCI_MOUSE,
};

static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn install(hwnd: HWND) -> Result<(), String> {
    TARGET_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("xuanci-mouse-hook".into())
        .stack_size(128 * 1024)
        .spawn(move || unsafe {
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) {
                Ok(hook) => {
                    let _ = tx.send(Ok(()));
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWindowsHookEx(hook);
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("SetWindowsHookExW: {e}")));
                }
            }
        })
        .map_err(|e| format!("创建鼠标钩子线程失败：{e}"))?;

    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "安装鼠标钩子超时".to_string())?
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 {
            let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let msg = wparam.0 as u32;

            match msg {
                WM_LBUTTONDOWN => post(info.pt, MOUSE_DOWN),
                WM_LBUTTONUP => post(info.pt, MOUSE_UP),
                WM_MOUSEWHEEL => post(info.pt, MOUSE_WHEEL),
                WM_RBUTTONDOWN | WM_MBUTTONDOWN => post(info.pt, MOUSE_OTHER_BUTTON),
                _ => {}
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

fn post(pt: POINT, event: usize) {
    let target = TARGET_HWND.load(Ordering::Relaxed);
    if target == 0 {
        return;
    }
    let packed = pack_point(pt.x, pt.y);
    unsafe {
        let _ = PostMessageW(
            Some(HWND(target as *mut core::ffi::c_void)),
            WM_XUANCI_MOUSE,
            WPARAM(event),
            LPARAM(packed),
        );
    }
}
