//! Borderless, non-activating toolbar drawn with GDI.

use std::mem::zeroed;

use windows::core::{w, PCWSTR, HSTRING};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    GetMonitorInfoW, InvalidateRect, MonitorFromPoint, SelectObject, SetBkMode, SetTextColor,
    BACKGROUND_MODE, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    DRAW_TEXT_FORMAT, DT_CENTER, DT_SINGLELINE, DT_VCENTER, HBRUSH, HFONT, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
    GetWindowRect, IsWindowVisible, KillTimer, PostMessageW, RegisterClassW, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, CREATESTRUCTW, GWLP_USERDATA, HWND_TOPMOST,
    MA_NOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WINDOW_EX_STYLE, WNDCLASSW,
    WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_NCCREATE, WM_PAINT, WM_TIMER, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const WM_MOUSELEAVE: u32 = 0x02A3;

use crate::messages::WM_XUANCI_ACTION;

const CLASS_NAME: &str = "XuanCiToolbarWindow";
const TIMER_AUTO_HIDE: usize = 1;
const AUTO_HIDE_MS: u32 = 5000;

const BTN_W_DIP: i32 = 58;
const PAD_DIP: i32 = 8;
const HEIGHT_DIP: i32 = 34;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

struct ToolbarState {
    app_hwnd: HWND,
    labels: Vec<String>,
    hover: Option<usize>,
    font: HFONT,
    font_dpi: u32,
    bg: HBRUSH,
    hover_bg: HBRUSH,
    border: HBRUSH,
}

impl ToolbarState {
    fn ensure_font(&mut self, dpi: u32) {
        if self.font_dpi == dpi && !self.font.0.is_null() {
            return;
        }
        unsafe {
            if !self.font.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
            let height = -((11 * dpi as i32) / 96).max(11);
            self.font = CreateFontW(
                height,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                DEFAULT_PITCH.0 as u32,
                w!("Segoe UI"),
            );
            self.font_dpi = dpi;
        }
    }
}

pub fn create(app_hwnd: HWND, labels: &[&str]) -> Result<HWND, String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
        let hinstance = HINSTANCE(instance.0);
        let class = HSTRING::from(CLASS_NAME);

        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = Some(toolbar_proc);
        wc.hInstance = hinstance;
        wc.lpszClassName = PCWSTR(class.as_ptr());

        let atom = RegisterClassW(&wc);
        if atom == 0 {
            let err = windows::Win32::Foundation::GetLastError();
            if err.0 != 1410 {
                return Err(format!("RegisterClassW(toolbar) failed: {}", err.0));
            }
        }

        let state = Box::new(ToolbarState {
            app_hwnd,
            labels: labels.iter().map(|s| s.to_string()).collect(),
            hover: None,
            font: HFONT::default(),
            font_dpi: 0,
            bg: CreateSolidBrush(rgb(250, 250, 250)),
            hover_bg: CreateSolidBrush(rgb(229, 241, 255)),
            border: CreateSolidBrush(rgb(210, 210, 210)),
        });

        let ex_style = WINDOW_EX_STYLE(
            WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0 | WS_EX_NOACTIVATE.0,
        );

        let hwnd = CreateWindowExW(
            ex_style,
            PCWSTR(class.as_ptr()),
            w!("XuanCi"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(hinstance),
            Some(Box::into_raw(state) as *const core::ffi::c_void),
        )
        .map_err(|e| format!("CreateWindowExW(toolbar): {e}"))?;

        Ok(hwnd)
    }
}

fn state(hwnd: HWND) -> Option<&'static mut ToolbarState> {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ToolbarState;
        ptr.as_mut()
    }
}

pub fn is_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

pub fn hide(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_AUTO_HIDE);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn contains_point(hwnd: HWND, x: i32, y: i32) -> bool {
    let mut rect = RECT::default();
    unsafe {
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
    }
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

/// Positions the toolbar near the cursor, clamped to the monitor work area.
pub fn show_near_cursor(hwnd: HWND, cursor_x: i32, cursor_y: i32) -> Result<(), String> {
    let Some(state) = state(hwnd) else {
        return Err("工具条状态丢失".into());
    };

    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: cursor_x,
                y: cursor_y,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        if dpi_x == 0 {
            dpi_x = GetDpiForWindow(hwnd).max(96);
        }
        state.ensure_font(dpi_x);

        let scale = dpi_x as f64 / 96.0;
        let count = state.labels.len().max(1) as i32;
        let width = ((count * BTN_W_DIP + PAD_DIP * 2) as f64 * scale).round() as i32;
        let height = (HEIGHT_DIP as f64 * scale).round() as i32;
        let gap = (10.0 * scale).round() as i32;
        let margin = (8.0 * scale).round() as i32;

        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let _ = GetMonitorInfoW(monitor, &mut mi);

        let mut x = cursor_x + gap;
        let mut y = cursor_y + gap;
        if x + width > mi.rcWork.right - margin {
            x = cursor_x - width - gap;
        }
        if y + height > mi.rcWork.bottom - margin {
            y = cursor_y - height - gap;
        }
        x = x.max(mi.rcWork.left + margin);
        y = y.max(mi.rcWork.top + margin);

        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .map_err(|e| format!("SetWindowPos(toolbar): {e}"))?;

        let _ = SetTimer(Some(hwnd), TIMER_AUTO_HIDE, AUTO_HIDE_MS, None);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }

    Ok(())
}

fn hit_test(state: &ToolbarState, hwnd: HWND, x: i32, y: i32) -> Option<usize> {
    let mut rc = RECT::default();
    unsafe {
        if GetClientRect(hwnd, &mut rc).is_err() {
            return None;
        }
    }
    if x < 0 || y < 0 || x >= rc.right || y >= rc.bottom {
        return None;
    }
    let count = state.labels.len();
    if count == 0 {
        return None;
    }
    let button_w = (rc.right / count as i32).max(1);
    Some(((x / button_w) as usize).min(count - 1))
}

unsafe extern "system" fn toolbar_proc(
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
                if let Some(state) = state(hwnd) {
                    state.ensure_font(GetDpiForWindow(hwnd).max(96));
                }
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                if let Some(state) = state(hwnd) {
                    let hit = hit_test(state, hwnd, x, y);
                    if hit != state.hover {
                        state.hover = hit;
                        let _ = InvalidateRect(Some(hwnd), None, true);
                    }
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                if let Some(state) = state(hwnd) {
                    state.hover = None;
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                if let Some(state) = state(hwnd) {
                    if let Some(index) = hit_test(state, hwnd, x, y) {
                        let app = state.app_hwnd;
                        hide(hwnd);
                        let _ = PostMessageW(Some(app), WM_XUANCI_ACTION, WPARAM(index), LPARAM(0));
                        return LRESULT(0);
                    }
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == TIMER_AUTO_HIDE {
                    hide(hwnd);
                }
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let dpi = (wparam.0 & 0xFFFF) as u32;
                if let Some(state) = state(hwnd) {
                    state.ensure_font(dpi.max(96));
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ToolbarState;
                if !ptr.is_null() {
                    let state = Box::from_raw(ptr);
                    let _ = DeleteObject(HGDIOBJ(state.bg.0));
                    let _ = DeleteObject(HGDIOBJ(state.hover_bg.0));
                    let _ = DeleteObject(HGDIOBJ(state.border.0));
                    if !state.font.0.is_null() {
                        let _ = DeleteObject(HGDIOBJ(state.font.0));
                    }
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn paint(hwnd: HWND) {
    unsafe {
        let mut ps: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let Some(state) = state(hwnd) else {
            let _ = EndPaint(hwnd, &ps);
            return;
        };

        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let _ = FillRect(hdc, &client, state.bg);
        let _ = SetBkMode(hdc, BACKGROUND_MODE(1));
        let old_font = SelectObject(hdc, HGDIOBJ(state.font.0));

        let count = state.labels.len() as i32;
        let button_w = (client.right / count.max(1)).max(1);

        for i in 0..count {
            let mut cell = RECT {
                left: i * button_w,
                top: 0,
                right: (i + 1) * button_w,
                bottom: client.bottom,
            };
            if state.hover == Some(i as usize) {
                let _ = FillRect(hdc, &cell, state.hover_bg);
            }
            let _ = SetTextColor(hdc, rgb(28, 28, 30));
            let mut label: Vec<u16> = state.labels[i as usize].encode_utf16().collect();
            let flags = DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0);
            let _ = DrawTextW(hdc, &mut label, &mut cell, flags);
        }

        let bottom = RECT {
            left: 0,
            top: (client.bottom - 1).max(0),
            right: client.right,
            bottom: client.bottom,
        };
        let _ = FillRect(hdc, &bottom, state.border);

        SelectObject(hdc, old_font);
        let _ = EndPaint(hwnd, &ps);
    }
}

/// Keeps the window class alive for the process lifetime.
#[allow(dead_code)]
pub fn destroy(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}
