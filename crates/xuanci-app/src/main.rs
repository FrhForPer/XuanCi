#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod pipeline;
mod storage;

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

/// Only one XuanCi may own the global hotkeys at a time.
fn acquire_single_instance() -> Result<Option<HANDLE>, String> {
    unsafe {
        let handle = CreateMutexW(None, true, w!("Local\\XuanCi-3F6C8C2E-9B4A-4C1D-8E7F-SingleInstance"))
            .map_err(|e| format!("CreateMutexW: {e}"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            Ok(None)
        } else {
            Ok(Some(handle))
        }
    }
}

fn main() {
    xuanci_win::dpi::enable_per_monitor_v2();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--autostart-enable") {
        if let Err(e) = autostart::set_enabled(true) {
            xuanci_win::dialog::error(&format!("开启开机自启失败：\n{e}"));
        }
        return;
    }
    if args.iter().any(|arg| arg == "--autostart-disable") {
        if let Err(e) = autostart::set_enabled(false) {
            xuanci_win::dialog::error(&format!("关闭开机自启失败：\n{e}"));
        }
        return;
    }

    let _instance_guard = match acquire_single_instance() {
        Ok(Some(handle)) => Some(handle),
        Ok(None) => {
            xuanci_win::dialog::info(
                "XuanCi 已经在运行。\n\n请在系统托盘区查看它的图标：\n右键可设置 API Key、模型，或退出。",
            );
            return;
        }
        Err(_) => None,
    };

    if let Err(e) = app::run() {
        xuanci_win::dialog::error(&format!("XuanCi 启动失败：\n{e}"));
    }
}
