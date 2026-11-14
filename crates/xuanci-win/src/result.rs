//! Streaming result window backed by a RichEdit control.

use std::mem::zeroed;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{w, PCWSTR, HSTRING};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetWindowLongPtrW,
    GetWindowRect, PostMessageW, RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowTextW, SetWindowPos, ShowWindow, CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA,
    HWND_TOP, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOW, WINDOW_EX_STYLE, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_DESTROY, WM_GETTEXT, WM_GETTEXTLENGTH, WM_NCCREATE, WM_SIZE,
    WINDOW_STYLE, WM_SETFONT, WNDCLASSW, WS_CHILD, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE, WS_VSCROLL,
};

use xuanci_core::ai::{ChatDelta, ChatRequest};
use xuanci_core::deepseek;
use xuanci_core::settings::AppSettings;

use crate::clipboard;
use crate::messages::{WM_XUANCI_RESULT_CHUNK, WM_XUANCI_RESULT_DONE};

const CLASS_NAME: &str = "XuanCiResultWindow";

const BTN_COPY: usize = 9001;
const BTN_STOP: usize = 9002;
const BTN_CLOSE: usize = 9003;

const EM_SETSEL: u32 = 0x00B1;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_SCROLLCARET: u32 = 0x00B7;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;
const BS_PUSHBUTTON: u32 = 0x0000;

const BUTTON_H: i32 = 30;

fn dbg(msg: &str) {
    if !cfg!(debug_assertions) {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::temp_dir().join("xuanci-debug.log"))
    {
        let _ = writeln!(f, "[result] {msg}");
    }
}
const MARGIN: i32 = 8;
const BAR_H: i32 = 46;
const BUTTON_W: i32 = 82;

#[derive(Default)]
struct StreamShared {
    pending: String,
    full: String,
    finished: bool,
    error: Option<String>,
}

struct ResultState {
    edit: HWND,
    copy_btn: HWND,
    stop_btn: HWND,
    close_btn: HWND,
    cancel: Arc<AtomicBool>,
    shared: Arc<Mutex<StreamShared>>,
    streaming: bool,
}

pub fn open(app_hwnd: HWND, title: &str) -> Result<HWND, String> {
    unsafe {
        let _ = LoadLibraryW(w!("Msftedit.dll"));
        let instance = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
        let hinstance = HINSTANCE(instance.0);
        let class = HSTRING::from(CLASS_NAME);

        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = Some(result_proc);
        wc.hInstance = hinstance;
        wc.lpszClassName = PCWSTR(class.as_ptr());
        let atom = RegisterClassW(&wc);
        if atom == 0 {
            let err = windows::Win32::Foundation::GetLastError();
            if err.0 != 1410 {
                return Err(format!("RegisterClassW(result) failed: {}", err.0));
            }
        }

        let state = Box::new(ResultState {
            edit: HWND::default(),
            copy_btn: HWND::default(),
            stop_btn: HWND::default(),
            close_btn: HWND::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            shared: Arc::new(Mutex::new(StreamShared::default())),
            streaming: false,
        });

        let title_w = HSTRING::from(title);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(title_w.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            560,
            None,
            None,
            Some(hinstance),
            Some(Box::into_raw(state) as *const core::ffi::c_void),
        )
        .map_err(|e| format!("CreateWindowExW(result): {e}"))?;
        dbg(&format!("created hwnd={:?}", hwnd));

        center_near_cursor(hwnd);
        dbg("centered");

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        dbg("shown");
        let _ = app_hwnd; // owner relationships are optional for v1
        Ok(hwnd)
    }
}

fn state(hwnd: HWND) -> Option<&'static mut ResultState> {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ResultState;
        ptr.as_mut()
    }
}

fn center_near_cursor(hwnd: HWND) {
    unsafe {
        let mut pt = Default::default();
        let _ = GetCursorPos(&mut pt);
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let _ = GetMonitorInfoW(monitor, &mut mi);

        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        let x = (mi.rcWork.left + mi.rcWork.right - w) / 2;
        let y = (mi.rcWork.top + mi.rcWork.bottom - h) / 2;
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
        );
    }
}

/// Clears the window and starts a streaming request on a worker thread.
pub fn start_stream(
    hwnd: HWND,
    title: &str,
    settings: AppSettings,
    api_key: String,
    request: ChatRequest,
) -> Result<(), String> {
    let state = state(hwnd).ok_or_else(|| "结果窗口状态丢失".to_string())?;

    let mut shared = state.shared.lock().map_err(|_| "状态锁失败".to_string())?;
    shared.pending.clear();
    shared.full.clear();
    shared.finished = false;
    shared.error = None;
    drop(shared);

    set_edit_text(state.edit, "");
    state.cancel.store(false, Ordering::Relaxed);
    state.streaming = true;
    set_title(hwnd, &format!("{title} · 生成中…"));

    let cancel = state.cancel.clone();
    let shared = state.shared.clone();
    let hwnd_value = hwnd.0 as isize;

    std::thread::Builder::new()
        .name("xuanci-ai".into())
        .stack_size(512 * 1024)
        .spawn(move || {
            let post = |msg: u32| {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd_value as *mut core::ffi::c_void)),
                        msg,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            };

            let mut sink = |delta: ChatDelta| match delta {
                ChatDelta::Content(text) => {
                    if let Ok(mut guard) = shared.lock() {
                        guard.pending.push_str(&text);
                        guard.full.push_str(&text);
                    }
                    post(WM_XUANCI_RESULT_CHUNK);
                }
                ChatDelta::Thinking(_) => {}
                ChatDelta::Done => {
                    if let Ok(mut guard) = shared.lock() {
                        guard.finished = true;
                    }
                    post(WM_XUANCI_RESULT_DONE);
                }
                ChatDelta::Error(error) => {
                    if let Ok(mut guard) = shared.lock() {
                        guard.error = Some(error);
                        guard.finished = true;
                    }
                    post(WM_XUANCI_RESULT_DONE);
                }
            };

            dbg("ai thread started");
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                deepseek::stream_chat(&settings, &api_key, &request, cancel, &mut sink)
            }));
            match outcome {
                Ok(Ok(())) => dbg("ai thread finished ok"),
                Ok(Err(error)) => {
                    dbg(&format!("ai thread error: {error}"));
                    if let Ok(mut guard) = shared.lock() {
                        if guard.error.is_none() {
                            guard.error = Some(error);
                        }
                        guard.finished = true;
                    }
                    post(WM_XUANCI_RESULT_DONE);
                }
                Err(_) => {
                    dbg("ai thread panicked");
                    if let Ok(mut guard) = shared.lock() {
                        if guard.error.is_none() {
                            guard.error = Some("AI 请求线程异常退出".to_string());
                        }
                        guard.finished = true;
                    }
                    post(WM_XUANCI_RESULT_DONE);
                }
            }
        })
        .map_err(|e| format!("启动 AI 线程失败：{e}"))?;

    Ok(())
}

fn set_title(hwnd: HWND, title: &str) {
    let text = HSTRING::from(title);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}

fn set_edit_text(edit: HWND, text: &str) {
    unsafe {
        let mut wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        SendMessageW(
            edit,
            windows::Win32::UI::WindowsAndMessaging::WM_SETTEXT,
            Some(WPARAM(0)),
            Some(LPARAM(wide.as_mut_ptr() as isize)),
        );
    }
}

fn append_edit_text(edit: HWND, text: &str) {
    if text.is_empty() {
        return;
    }
    unsafe {
        let len = SendMessageW(
            edit,
            WM_GETTEXTLENGTH,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        )
        .0;
        SendMessageW(
            edit,
            EM_SETSEL,
            Some(WPARAM(len as usize)),
            Some(LPARAM(len as isize)),
        );
        let mut wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        SendMessageW(
            edit,
            EM_REPLACESEL,
            Some(WPARAM(0)),
            Some(LPARAM(wide.as_mut_ptr() as isize)),
        );
        SendMessageW(edit, EM_SCROLLCARET, Some(WPARAM(0)), Some(LPARAM(0)));
    }
}

fn read_edit_text(edit: HWND) -> String {
    unsafe {
        let len = SendMessageW(
            edit,
            WM_GETTEXTLENGTH,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        )
        .0;
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        SendMessageW(
            edit,
            WM_GETTEXT,
            Some(WPARAM(buf.len())),
            Some(LPARAM(buf.as_mut_ptr() as isize)),
        );
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

fn layout(hwnd: HWND) {
    let Some(state) = state(hwnd) else {
        return;
    };
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    let width = client.right - client.left;
    let height = client.bottom - client.top;
    let edit_w = (width - MARGIN * 2).max(10);
    let edit_h = (height - BAR_H - MARGIN).max(10);

    unsafe {
        let _ = SetWindowPos(
            state.edit,
            Some(HWND_TOP),
            MARGIN,
            MARGIN,
            edit_w,
            edit_h,
            SWP_NOZORDER,
        );

        let by = height - BUTTON_H - 10;
        let mut bx = width - MARGIN - BUTTON_W;
        let _ = SetWindowPos(state.close_btn, Some(HWND_TOP), bx, by, BUTTON_W, BUTTON_H, SWP_NOZORDER);
        bx -= BUTTON_W + 8;
        let _ = SetWindowPos(state.stop_btn, Some(HWND_TOP), bx, by, BUTTON_W, BUTTON_H, SWP_NOZORDER);
        bx -= BUTTON_W + 8;
        let _ = SetWindowPos(state.copy_btn, Some(HWND_TOP), bx, by, BUTTON_W, BUTTON_H, SWP_NOZORDER);
    }
}

fn create_controls(hwnd: HWND, state: &mut ResultState, hinstance: HINSTANCE) {
    let edit_style = WINDOW_STYLE(
        WS_CHILD.0 | WS_VISIBLE.0 | WS_VSCROLL.0 | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
    );
    unsafe {
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("RICHEDIT50W"),
            w!(""),
            edit_style,
            0,
            0,
            10,
            10,
            Some(hwnd),
            None,
            Some(hinstance),
            None,
        )
        .unwrap_or_default();
        state.edit = edit;

        let button_style = WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | BS_PUSHBUTTON);
        let make_button = |label: PCWSTR, id: usize| -> HWND {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                label,
                button_style,
                0,
                0,
                10,
                10,
                Some(hwnd),
                Some(windows::Win32::UI::WindowsAndMessaging::HMENU(
                    id as *mut core::ffi::c_void,
                )),
                Some(hinstance),
                None,
            )
            .unwrap_or_default()
        };
        state.copy_btn = make_button(w!("复制"), BTN_COPY);
        state.stop_btn = make_button(w!("停止"), BTN_STOP);
        state.close_btn = make_button(w!("关闭"), BTN_CLOSE);

        let font = windows::Win32::Graphics::Gdi::GetStockObject(
            windows::Win32::Graphics::Gdi::DEFAULT_GUI_FONT,
        );
        for control in [state.edit, state.copy_btn, state.stop_btn, state.close_btn] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }
}

unsafe extern "system" fn result_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_NCCREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                LRESULT(1)
            }
            WM_CREATE => {
                dbg("WM_CREATE");
                let instance = GetModuleHandleW(None).unwrap_or_default();
                let hinstance = HINSTANCE(instance.0);
                if let Some(state) = state(hwnd) {
                    create_controls(hwnd, state, hinstance);
                }
                layout(hwnd);
                LRESULT(0)
            }
            WM_SIZE => {
                layout(hwnd);
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wparam.0 & 0xFFFF;
                if id == BTN_COPY {
                    if let Some(state) = state(hwnd) {
                        let text = read_edit_text(state.edit);
                        if !text.is_empty() {
                            if let Err(e) = clipboard::set_text(&text) {
                                set_title(hwnd, &format!("复制失败：{e}"));
                            }
                        }
                    }
                } else if id == BTN_STOP {
                    if let Some(state) = state(hwnd) {
                        state.cancel.store(true, Ordering::Relaxed);
                        if state.streaming {
                            set_title(hwnd, "已停止");
                        }
                    }
                } else if id == BTN_CLOSE {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_XUANCI_RESULT_CHUNK => {
                if let Some(state) = state(hwnd) {
                    let pending = state
                        .shared
                        .lock()
                        .ok()
                        .map(|mut guard| std::mem::take(&mut guard.pending))
                        .unwrap_or_default();
                    append_edit_text(state.edit, &pending);
                }
                LRESULT(0)
            }
            WM_XUANCI_RESULT_DONE => {
                if let Some(state) = state(hwnd) {
                    let (error, full) = state
                        .shared
                        .lock()
                        .map(|guard| (guard.error.clone(), guard.full.clone()))
                        .unwrap_or_default();
                    state.streaming = false;
                    state.cancel.store(true, Ordering::Relaxed);
                    if let Some(error) = error {
                        if full.is_empty() {
                            append_edit_text(state.edit, &format!("请求失败：{error}"));
                        }
                        set_title(hwnd, &format!("XuanCi · {error}"));
                    } else {
                        set_title(hwnd, "XuanCi · 完成");
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                dbg("WM_CLOSE");
                if let Some(state) = state(hwnd) {
                    state.cancel.store(true, Ordering::Relaxed);
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                dbg("WM_DESTROY");
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ResultState;
                if !ptr.is_null() {
                    let state = Box::from_raw(ptr);
                    state.cancel.store(true, Ordering::Relaxed);
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
