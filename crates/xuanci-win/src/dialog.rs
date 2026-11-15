//! Minimal message box and input box helpers.

use std::cell::RefCell;
use std::mem::zeroed;
use std::path::PathBuf;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OPENFILENAMEW, OPEN_FILENAME_FLAGS, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
    OFN_PATHMUSTEXIST,
};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, GetStockObject, MonitorFromPoint, DEFAULT_GUI_FONT, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, HMENU, IsWindow, IsWindowVisible,
    MessageBoxW, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
    SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
    TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, ES_AUTOHSCROLL, ES_AUTOVSCROLL,
    ES_MULTILINE, ES_PASSWORD, ES_WANTRETURN, GWLP_USERDATA, HWND_TOP, MSG, MB_ICONERROR,
    IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO, SWP_NOSIZE, SWP_NOZORDER,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_DESTROY,
    WM_GETTEXT, WM_GETTEXTLENGTH, WM_KEYDOWN, WM_NCCREATE, WM_SETFONT, WM_SETTEXT, WNDCLASSW,
    WS_BORDER, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_VISIBLE, WS_VSCROLL,
};

thread_local! {
    static INPUT_RESULT: RefCell<Option<String>> = const { RefCell::new(None) };
    static SELECT_RESULT: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn info(text: &str) {
    let text = HSTRING::from(text);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("XuanCi"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

pub fn error(text: &str) {
    let text = HSTRING::from(text);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("XuanCi - 错误"),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub fn confirm(text: &str) -> bool {
    let text = HSTRING::from(text);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("XuanCi"),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    }
}

/// Native "Open file" dialog. Returns None when cancelled.
pub fn open_file(parent: HWND, title: &str) -> Option<PathBuf> {
    unsafe {
        let mut buffer = vec![0u16; 32768];
        let filter: Vec<u16> = "可执行文件 (*.exe)\0*.exe\0所有文件 (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
        let title_h = HSTRING::from(title);
        let mut ofn = OPENFILENAMEW::default();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = parent;
        ofn.lpstrFilter = PCWSTR(filter.as_ptr());
        ofn.lpstrFile = PWSTR(buffer.as_mut_ptr());
        ofn.nMaxFile = buffer.len() as u32;
        ofn.lpstrTitle = PCWSTR(title_h.as_ptr());
        ofn.Flags = OPEN_FILENAME_FLAGS(
            OFN_FILEMUSTEXIST.0 | OFN_PATHMUSTEXIST.0 | OFN_HIDEREADONLY.0,
        );
        if !GetOpenFileNameW(&mut ofn).as_bool() {
            return None;
        }
        let length = buffer.iter().position(|&value| value == 0).unwrap_or(0);
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
    }
}

pub struct InputSpec<'a> {
    pub title: &'a str,
    pub label: &'a str,
    pub initial: &'a str,
    pub password: bool,
    pub multiline: bool,
}

struct InputState {
    edit: HWND,
}

const INPUT_CLASS: &str = "XuanCiInputBox";
const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;

/// Shows a small modal input box. Returns None when cancelled.
pub fn input_text(parent: HWND, spec: InputSpec<'_>) -> Option<String> {
    INPUT_RESULT.with(|slot| *slot.borrow_mut() = None);

    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let hinstance = HINSTANCE(instance.0);
        let class = HSTRING::from(INPUT_CLASS);
        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = Some(input_proc);
        wc.hInstance = hinstance;
        wc.lpszClassName = PCWSTR(class.as_ptr());
        let atom = RegisterClassW(&wc);
        if atom == 0 {
            let err = windows::Win32::Foundation::GetLastError();
            if err.0 != 1410 {
                return None;
            }
        }

        let height = if spec.multiline { 300 } else { 190 };
        let title = HSTRING::from(spec.title);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WINDOW_STYLE(WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_VISIBLE.0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            480,
            height,
            Some(parent),
            None,
            Some(hinstance),
            None,
        )
        .ok()?;
        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));

        let state = Box::new(InputState {
            edit: HWND::default(),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

        let label_w = HSTRING::from(spec.label);
        let label = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(label_w.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            16,
            14,
            430,
            40,
            Some(hwnd),
            None,
            Some(hinstance),
            None,
        )
        .unwrap_or_default();

        let mut edit_style = WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32;
        if spec.password {
            edit_style |= ES_PASSWORD as u32;
        }
        if spec.multiline {
            edit_style |= ES_MULTILINE as u32
                | ES_AUTOVSCROLL as u32
                | ES_WANTRETURN as u32
                | WS_VSCROLL.0;
        }
        let edit_h = if spec.multiline { 150 } else { 26 };
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!(""),
            WINDOW_STYLE(edit_style),
            16,
            56,
            430,
            edit_h,
            Some(hwnd),
            None,
            Some(hinstance),
            None,
        )
        .unwrap_or_default();

        if let Some(state) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut InputState).as_mut() {
            state.edit = edit;
        }
        let initial = HSTRING::from(spec.initial);
        let _ = SendMessageW(
            edit,
            WM_SETTEXT,
            Some(WPARAM(0)),
            Some(LPARAM(initial.as_ptr() as isize)),
        );

        let button_y = height - 78;
        let ok = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("确定"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            260,
            button_y,
            88,
            30,
            Some(hwnd),
            Some(HMENU(ID_OK as *mut core::ffi::c_void)),
            Some(hinstance),
            None,
        )
        .unwrap_or_default();
        let cancel = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("取消"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            358,
            button_y,
            88,
            30,
            Some(hwnd),
            Some(HMENU(ID_CANCEL as *mut core::ffi::c_void)),
            Some(hinstance),
            None,
        )
        .unwrap_or_default();

        let font = GetStockObject(DEFAULT_GUI_FONT);
        for control in [label, edit, ok, cancel] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }

        center_on_parent(hwnd, parent);
        let _ = EnableWindow(parent, false);
        let _ = SetForegroundWindow(hwnd);
        SendMessageW(edit, 0x00B7, Some(WPARAM(0)), Some(LPARAM(0))); // EM_SCROLLCARET

        let mut msg = MSG::default();
        while IsWindow(Some(hwnd)).as_bool() {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                break;
            }
            let is_edit_key = msg.hwnd == edit && msg.message == WM_KEYDOWN;
            if is_edit_key {
                let vk = msg.wParam.0 as u32;
                if vk == VK_RETURN.0 as u32 && !spec.multiline {
                    finish_input(hwnd, true);
                    continue;
                }
                if vk == VK_ESCAPE.0 as u32 {
                    finish_input(hwnd, false);
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = EnableWindow(parent, true);
        let _ = SetForegroundWindow(parent);
    }

    INPUT_RESULT.with(|slot| slot.borrow_mut().take())
}

fn center_on_parent(hwnd: HWND, parent: HWND) {
    unsafe {
        let mut ar = RECT::default();
        let _ = GetWindowRect(hwnd, &mut ar);
        let w = ar.right - ar.left;
        let h = ar.bottom - ar.top;

        // The app's hidden message window sits at (0,0) with zero size, so
        // centering on it would place the dialog half off-screen. Only use the
        // parent when it is a real, visible window.
        let mut pr = RECT::default();
        let parent_visible = !parent.0.is_null()
            && IsWindowVisible(parent).as_bool()
            && GetWindowRect(parent, &mut pr).is_ok()
            && pr.right > pr.left
            && pr.bottom > pr.top;

        let (anchor_x, anchor_y) = if parent_visible {
            ((pr.left + pr.right) / 2, (pr.top + pr.bottom) / 2)
        } else {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            (pt.x, pt.y)
        };

        let monitor = MonitorFromPoint(
            POINT {
                x: anchor_x,
                y: anchor_y,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let work = if GetMonitorInfoW(monitor, &mut mi).as_bool()
            && mi.rcWork.right > mi.rcWork.left
            && mi.rcWork.bottom > mi.rcWork.top
        {
            mi.rcWork
        } else {
            RECT {
                left: 0,
                top: 0,
                right: GetSystemMetrics(SM_CXSCREEN),
                bottom: GetSystemMetrics(SM_CYSCREEN),
            }
        };

        let margin = 8;
        let min_x = work.left + margin;
        let max_x = (work.right - w - margin).max(min_x);
        let min_y = work.top + margin;
        let max_y = (work.bottom - h - margin).max(min_y);
        let x = (anchor_x - w / 2).max(min_x).min(max_x);
        let y = (anchor_y - h / 2).max(min_y).min(max_y);

        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            x,
            y,
            w,
            h,
            SWP_NOSIZE | SWP_NOZORDER,
        );
    }
}

fn finish_input(hwnd: HWND, ok: bool) {
    unsafe {
        if ok {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut InputState;
            if let Some(state) = state_ptr.as_ref() {
                let text = read_control_text(state.edit);
                INPUT_RESULT.with(|slot| *slot.borrow_mut() = Some(text));
            }
        }
        let _ = DestroyWindow(hwnd);
    }
}

fn read_control_text(control: HWND) -> String {
    unsafe {
        let len = SendMessageW(
            control,
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
            control,
            WM_GETTEXT,
            Some(WPARAM(buf.len())),
            Some(LPARAM(buf.as_mut_ptr() as isize)),
        );
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

const SELECT_CLASS: &str = "XuanCiSelectBox";

const LB_ADDSTRING: u32 = 0x0180;
const LB_SETCURSEL: u32 = 0x0186;
const LB_GETCURSEL: u32 = 0x0188;
const LB_GETTEXT: u32 = 0x0189;
const LB_GETTEXTLEN: u32 = 0x018A;
const LBS_NOTIFY: u32 = 0x0001;
const LBN_DBLCLK: u32 = 2;

struct SelectState {
    list: HWND,
}

/// Shows a modal list selection dialog. Returns None when cancelled.
pub fn select_item(
    parent: HWND,
    title: &str,
    label: &str,
    items: &[String],
) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    SELECT_RESULT.with(|slot| *slot.borrow_mut() = None);

    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let hinstance = HINSTANCE(instance.0);
        let class = HSTRING::from(SELECT_CLASS);
        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = Some(select_proc);
        wc.hInstance = hinstance;
        wc.lpszClassName = PCWSTR(class.as_ptr());
        let atom = RegisterClassW(&wc);
        if atom == 0 {
            let err = windows::Win32::Foundation::GetLastError();
            if err.0 != 1410 {
                return None;
            }
        }

        let title_h = HSTRING::from(title);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(title_h.as_ptr()),
            WINDOW_STYLE(WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_VISIBLE.0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            480,
            480,
            Some(parent),
            None,
            Some(hinstance),
            None,
        )
        .ok()?;
        let _ = SetWindowTextW(hwnd, PCWSTR(title_h.as_ptr()));

        let state = Box::new(SelectState {
            list: HWND::default(),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

        let label_h = HSTRING::from(label);
        let label_w = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(label_h.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            16,
            14,
            430,
            36,
            Some(hwnd),
            None,
            Some(hinstance),
            None,
        )
        .unwrap_or_default();

        let list = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("LISTBOX"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | WS_VSCROLL.0 | LBS_NOTIFY),
            16,
            56,
            430,
            300,
            Some(hwnd),
            None,
            Some(hinstance),
            None,
        )
        .unwrap_or_default();

        if let Some(state) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SelectState).as_mut() {
            state.list = list;
        }

        for item in items {
            let mut wide: Vec<u16> = item.encode_utf16().chain(std::iter::once(0)).collect();
            SendMessageW(
                list,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(wide.as_mut_ptr() as isize)),
            );
        }
        SendMessageW(list, LB_SETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)));

        let ok = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("确定"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            260,
            400,
            88,
            30,
            Some(hwnd),
            Some(HMENU(ID_OK as *mut core::ffi::c_void)),
            Some(hinstance),
            None,
        )
        .unwrap_or_default();
        let cancel = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("取消"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            358,
            400,
            88,
            30,
            Some(hwnd),
            Some(HMENU(ID_CANCEL as *mut core::ffi::c_void)),
            Some(hinstance),
            None,
        )
        .unwrap_or_default();

        let font = GetStockObject(DEFAULT_GUI_FONT);
        for control in [label_w, list, ok, cancel] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }

        center_on_parent(hwnd, parent);
        let _ = EnableWindow(parent, false);
        let _ = SetForegroundWindow(hwnd);

        let mut msg = MSG::default();
        while IsWindow(Some(hwnd)).as_bool() {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                break;
            }
            if msg.hwnd == list && msg.message == WM_KEYDOWN {
                let vk = msg.wParam.0 as u32;
                if vk == VK_RETURN.0 as u32 {
                    finish_select(hwnd, true);
                    continue;
                }
                if vk == VK_ESCAPE.0 as u32 {
                    finish_select(hwnd, false);
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = EnableWindow(parent, true);
        let _ = SetForegroundWindow(parent);
    }

    SELECT_RESULT.with(|slot| slot.borrow_mut().take())
}

fn finish_select(hwnd: HWND, ok: bool) {
    unsafe {
        if ok {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SelectState;
            if let Some(state) = ptr.as_ref() {
                let index =
                    SendMessageW(state.list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0;
                if index >= 0 {
                    let len = SendMessageW(
                        state.list,
                        LB_GETTEXTLEN,
                        Some(WPARAM(index as usize)),
                        Some(LPARAM(0)),
                    )
                    .0;
                    if len >= 0 {
                        let mut buf = vec![0u16; len as usize + 1];
                        SendMessageW(
                            state.list,
                            LB_GETTEXT,
                            Some(WPARAM(index as usize)),
                            Some(LPARAM(buf.as_mut_ptr() as isize)),
                        );
                        let text = String::from_utf16_lossy(&buf[..len as usize]);
                        SELECT_RESULT.with(|slot| *slot.borrow_mut() = Some(text));
                    }
                }
            }
        }
        let _ = DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn select_proc(
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
            WM_COMMAND => {
                let id = wparam.0 & 0xFFFF;
                let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if id == ID_OK {
                    finish_select(hwnd, true);
                } else if id == ID_CANCEL {
                    finish_select(hwnd, false);
                } else if code == LBN_DBLCLK {
                    finish_select(hwnd, true);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SelectState;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe extern "system" fn input_proc(
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
            WM_COMMAND => {
                match wparam.0 & 0xFFFF {
                    ID_OK => finish_input(hwnd, true),
                    ID_CANCEL => finish_input(hwnd, false),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut InputState;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
