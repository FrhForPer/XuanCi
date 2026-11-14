//! Tracks the last foreground window that does not belong to XuanCi.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::Accessibility::{
    SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, GetWindowThreadProcessId, PostMessageW, TranslateMessage, MSG,
    EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT,
};

use crate::messages::WM_XUANCI_FOREGROUND;

static LAST_EXTERNAL: AtomicIsize = AtomicIsize::new(0);
static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn last_external_foreground() -> isize {
    LAST_EXTERNAL.load(Ordering::Relaxed)
}

pub fn install(hwnd: HWND) -> Result<(), String> {
    TARGET_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("xuanci-win-event".into())
        .stack_size(128 * 1024)
        .spawn(move || unsafe {
            let hook: HWINEVENTHOOK = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            if hook.is_invalid() {
                let _ = tx.send(Err("SetWinEventHook 失败".to_string()));
                return;
            }
            let _ = tx.send(Ok(()));

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWinEvent(hook);
        })
        .map_err(|e| format!("创建前台追踪线程失败：{e}"))?;

    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "安装前台追踪超时".to_string())?
}

unsafe extern "system" fn event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _idobject: i32,
    _idchild: i32,
    _thread: u32,
    _time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND || hwnd.0.is_null() {
        return;
    }

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid != 0 && pid != std::process::id() {
        LAST_EXTERNAL.store(hwnd.0 as isize, Ordering::Relaxed);
    }

    let target = TARGET_HWND.load(Ordering::Relaxed);
    if target != 0 {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(target as *mut core::ffi::c_void)),
                WM_XUANCI_FOREGROUND,
                Default::default(),
                LPARAM(0),
            );
        }
    }
}
