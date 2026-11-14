//! Foreground window snapshot and target classification.

use std::path::Path;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId,
};

const INTEGRITY_MEDIUM: u32 = 0x2000;
const INTEGRITY_HIGH: u32 = 0x3000;

#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub process_name: String,
    pub class_name: String,
    pub integrity: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Normal,
    SelfProcess,
    Console,
    IdeTerminal,
    Explorer,
    Elevated,
    Unknown,
}

pub fn cursor_position() -> (i32, i32) {
    unsafe {
        let mut pt = windows::Win32::Foundation::POINT::default();
        if GetCursorPos(&mut pt).is_ok() {
            (pt.x, pt.y)
        } else {
            (0, 0)
        }
    }
}

pub fn foreground_target() -> Option<TargetInfo> {
    unsafe { target_for_hwnd(GetForegroundWindow()) }
}

pub fn target_for_hwnd(hwnd: HWND) -> Option<TargetInfo> {
    unsafe {
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf).max(0) as usize;
        let class_name = String::from_utf16_lossy(&class_buf[..class_len]);

        let process_name = process_image_name(pid).unwrap_or_default();
        let integrity = process_integrity(pid);

        Some(TargetInfo {
            hwnd: hwnd.0 as isize,
            pid,
            process_name,
            class_name,
            integrity,
        })
    }
}

fn process_image_name(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(process);
        result.ok()?;
        let full = String::from_utf16_lossy(&buf[..size as usize]);
        Path::new(&full)
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
    }
}

fn process_integrity(pid: u32) -> Option<u32> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let level = token_integrity(process);
        let _ = CloseHandle(process);
        level
    }
}

fn current_integrity() -> Option<u32> {
    unsafe { token_integrity(GetCurrentProcess()) }
}

unsafe fn token_integrity(process: HANDLE) -> Option<u32> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;

        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }

        let words = (needed as usize).div_ceil(std::mem::size_of::<usize>()) + 1;
        let mut buf = vec![0usize; words];
        let result = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        );
        if result.is_err() {
            let _ = CloseHandle(token);
            return None;
        }

        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            let _ = CloseHandle(token);
            return None;
        }
        let rid = *GetSidSubAuthority(sid, (count - 1) as u32);
        let _ = CloseHandle(token);
        Some(rid)
    }
}

fn is_elevated(info: &TargetInfo) -> bool {
    let target = info.integrity.unwrap_or(INTEGRITY_MEDIUM);
    let current = current_integrity().unwrap_or(INTEGRITY_MEDIUM);
    target >= INTEGRITY_HIGH && current < INTEGRITY_HIGH
}

fn is_console(process: &str, class: &str) -> bool {
    const CONSOLE_PROCESSES: &[&str] = &[
        "conhost.exe",
        "windowsterminal.exe",
        "openconsole.exe",
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "wsl.exe",
        "mintty.exe",
        "wezterm-gui.exe",
        "alacritty.exe",
        "windowsterminalpreview.exe",
    ];
    const CONSOLE_CLASSES: &[&str] = &[
        "consolewindowclass",
        "cascadia_hosting_window_class",
        "pseudoconsolewindow",
        "mintty",
        "virtualconsoleclass",
    ];
    CONSOLE_PROCESSES.contains(&process) || CONSOLE_CLASSES.contains(&class.to_ascii_lowercase().as_str())
}

fn is_ide(process: &str) -> bool {
    const IDE_PROCESSES: &[&str] = &[
        "code.exe",
        "code - insiders.exe",
        "idea64.exe",
        "pycharm64.exe",
        "webstorm64.exe",
        "rider64.exe",
        "goland64.exe",
        "clion64.exe",
        "devenv.exe",
    ];
    IDE_PROCESSES.contains(&process)
}

pub fn classify(info: &TargetInfo) -> TargetKind {
    if info.pid == std::process::id() {
        return TargetKind::SelfProcess;
    }
    if info.process_name.is_empty() && info.integrity.is_none() {
        return TargetKind::Unknown;
    }
    if is_elevated(info) {
        return TargetKind::Elevated;
    }
    if is_console(&info.process_name, &info.class_name) {
        return TargetKind::Console;
    }
    if is_ide(&info.process_name) {
        return TargetKind::IdeTerminal;
    }
    if info.process_name == "explorer.exe" {
        return TargetKind::Explorer;
    }
    TargetKind::Normal
}

/// Whether Ctrl+C fallback is safe for this target.
pub fn allows_clipboard_fallback(kind: TargetKind) -> bool {
    matches!(kind, TargetKind::Normal)
}
