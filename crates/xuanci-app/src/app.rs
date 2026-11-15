//! Application bootstrap, main message loop and UI wiring.

use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW,
    PostMessageW, PostQuitMessage, SetForegroundWindow, TrackPopupMenu, TranslateMessage,
    MENU_ITEM_FLAGS, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN,
    TPM_RIGHTBUTTON, WM_COMMAND, WM_DESTROY, WM_HOTKEY,
};

use xuanci_core::ai::ChatRequest;
use xuanci_core::prompt;
use xuanci_core::selection::SelectionContext;
use xuanci_core::settings::{detect_provider, AppSettings, ProviderProfile};
use xuanci_core::templates::PromptTemplate;
use xuanci_win::dialog::{self, InputSpec};
use xuanci_win::messages::{
    self, MOUSE_DOWN, MOUSE_OTHER_BUTTON, MOUSE_UP, MOUSE_WHEEL, WM_XUANCI_ACTION,
    WM_XUANCI_CAPTURED, WM_XUANCI_CAPTURE_FAILED, WM_XUANCI_FOREGROUND, WM_XUANCI_MODELS_READY,
    WM_XUANCI_MOUSE,
};
use xuanci_win::{foreground, hooks, hotkey, result, toolbar, tray, window};

use crate::autostart;
use crate::pipeline::{self, TriggerMode};
use crate::storage;

const HOTKEY_TRANSLATE_ID: i32 = 0x1001;
const HOTKEY_ASK_AI_ID: i32 = 0x1002;

/// Debug-only hotkey (Ctrl+Alt+C) that always exercises the clipboard fallback.
#[cfg(debug_assertions)]
const HOTKEY_CLIPBOARD_TEST_ID: i32 = 0x10F1;

const MENU_TEST_CAPTURE: usize = 1;
const MENU_EXIT: usize = 2;
const MENU_SET_KEY: usize = 3;
const MENU_AUTOSTART: usize = 6;
const MENU_AUTO_FALLBACK: usize = 7;
const MENU_PROVIDER_ADD: usize = 10;
const MENU_PROVIDER_EDIT: usize = 11;
const MENU_PROVIDER_DELETE: usize = 12;
const MENU_FETCH_MODELS: usize = 13;
const MENU_PROVIDER_BASE: usize = 200;

const ACTION_TRANSLATE: usize = 0;
const ACTION_ASK_AI: usize = 1;
/// Used by the Ctrl+Alt+Q hotkey: ask the model to explain the selection.
const ACTION_ASK_QUICK: usize = 2;

static APP_HWND: AtomicIsize = AtomicIsize::new(0);
static TOOLBAR_HWND: AtomicIsize = AtomicIsize::new(0);
static LAST_SELECTION: Mutex<Option<SelectionContext>> = Mutex::new(None);
static MOUSE_DOWN_AT: Mutex<Option<(i32, i32, Instant)>> = Mutex::new(None);
static LAST_CLICK: Mutex<Option<(i32, i32, Instant)>> = Mutex::new(None);
static TEMPLATES: OnceLock<Mutex<Vec<PromptTemplate>>> = OnceLock::new();
static API_KEYS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
static MODELS_RESULT: Mutex<Option<(String, Result<Vec<String>, String>)>> = Mutex::new(None);

const DRAG_THRESHOLD_PX: i32 = 8;
const DRAG_MIN_MS: u128 = 60;
const DOUBLE_CLICK_MS: u128 = 500;
const DOUBLE_CLICK_SLOP_PX: i32 = 6;

fn hwnd_from(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}

fn app_hwnd() -> HWND {
    hwnd_from(APP_HWND.load(Ordering::Relaxed))
}

fn settings_cell() -> &'static Mutex<AppSettings> {
    static CELL: OnceLock<Mutex<AppSettings>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(load_settings()))
}

fn load_settings() -> AppSettings {
    #[allow(unused_mut)]
    let mut settings = storage::load_settings();
    #[cfg(debug_assertions)]
    {
        if let Ok(url) = std::env::var("XUANCI_DEBUG_BASE_URL") {
            settings.base_url = url;
        }
        if let Ok(model) = std::env::var("XUANCI_DEBUG_MODEL") {
            settings.model = model;
        }
    }
    settings
}

fn settings_snapshot() -> AppSettings {
    settings_cell()
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn save_settings_cell(settings: &AppSettings) -> Result<(), String> {
    if let Ok(mut cell) = settings_cell().lock() {
        *cell = settings.clone();
    }
    storage::save_settings(settings)
}

fn api_keys_cell() -> &'static Mutex<HashMap<String, String>> {
    API_KEYS.get_or_init(|| {
        let settings = settings_snapshot();
        let mut keys = storage::load_api_keys(&settings);
        if !keys.contains_key(&settings.active_provider) {
            if let Some(key) = debug_api_key() {
                keys.insert(settings.active_provider.clone(), key);
            }
        }
        Mutex::new(keys)
    })
}

#[cfg(debug_assertions)]
fn debug_api_key() -> Option<String> {
    std::env::var("XUANCI_DEBUG_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
}

#[cfg(not(debug_assertions))]
fn debug_api_key() -> Option<String> {
    None
}

fn active_api_key() -> Option<String> {
    let settings = settings_snapshot();
    key_for(&settings.active_provider)
}

fn key_for(provider_id: &str) -> Option<String> {
    api_keys_cell()
        .lock()
        .ok()
        .and_then(|keys| keys.get(provider_id).cloned())
}

fn set_key_for(provider_id: &str, key: &str) -> Result<(), String> {
    let mut keys = api_keys_cell()
        .lock()
        .map_err(|_| "密钥表锁失败".to_string())?;
    keys.insert(provider_id.to_string(), key.to_string());
    storage::save_api_keys(&keys)
}

fn remove_key_for(provider_id: &str) {
    if let Ok(mut keys) = api_keys_cell().lock() {
        keys.remove(provider_id);
        let _ = storage::save_api_keys(&keys);
    }
}

fn templates_cell() -> &'static Mutex<Vec<PromptTemplate>> {
    TEMPLATES.get_or_init(|| Mutex::new(storage::load_templates()))
}

fn template_snapshot() -> Vec<PromptTemplate> {
    templates_cell()
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn take_selection() -> Option<SelectionContext> {
    LAST_SELECTION.lock().ok().and_then(|mut slot| slot.take())
}

fn base_url_allowed(url: &str) -> bool {
    url.starts_with("https://")
        || url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost")
}

pub fn run() -> Result<(), String> {
    let _ = settings_cell();
    let _ = api_keys_cell();
    debug_log(&format!(
        "config loaded providers={} active={} key={} endpoint={}",
        settings_snapshot().providers.len(),
        settings_snapshot().active_name(),
        active_api_key().is_some(),
        settings_snapshot().endpoint()
    ));

    let window = window::create("XuanCiMessageWindow", "XuanCi", Some(main_wndproc))?;
    APP_HWND.store(window.hwnd.0 as isize, Ordering::SeqCst);

    let templates = template_snapshot();
    let mut labels: Vec<&str> = vec!["翻译", "AI 问答"];
    labels.extend(templates.iter().map(|item| item.name.as_str()));
    let toolbar_hwnd = toolbar::create(window.hwnd, &labels)?;
    TOOLBAR_HWND.store(toolbar_hwnd.0 as isize, Ordering::SeqCst);
    debug_log(&format!("toolbar created with {} actions", labels.len()));

    hooks::install(window.hwnd)?;
    debug_log("mouse hook installed");
    foreground::install(window.hwnd)?;
    debug_log("foreground hook installed");

    tray::add(window.hwnd, "XuanCi 划词助手")
        .map_err(|e| format!("托盘图标创建失败：{e}"))?;

    let mods = hotkey::MOD_CONTROL | hotkey::MOD_ALT | hotkey::MOD_NOREPEAT;
    if let Err(e) = hotkey::register(window.hwnd, HOTKEY_TRANSLATE_ID, mods, 0x44) {
        dialog::error(&format!("注册 Ctrl+Alt+D 失败：{e:?}"));
    }
    if let Err(e) = hotkey::register(window.hwnd, HOTKEY_ASK_AI_ID, mods, 0x51) {
        dialog::error(&format!("注册 Ctrl+Alt+Q 失败：{e:?}"));
    }
    #[cfg(debug_assertions)]
    if let Err(e) = hotkey::register(window.hwnd, HOTKEY_CLIPBOARD_TEST_ID, mods, 0x43) {
        dialog::error(&format!("注册 Ctrl+Alt+C 失败：{e:?}"));
    }

    let autostart = std::env::args().any(|arg| arg == "--autostart");
    if !autostart && active_api_key().is_none() {
        prompt_api_key(window.hwnd);
    }

    debug_log("entering message loop");
    message_loop();

    hotkey::unregister(window.hwnd, HOTKEY_TRANSLATE_ID);
    hotkey::unregister(window.hwnd, HOTKEY_ASK_AI_ID);
    #[cfg(debug_assertions)]
    hotkey::unregister(window.hwnd, HOTKEY_CLIPBOARD_TEST_ID);
    tray::remove(window.hwnd);
    Ok(())
}

fn message_loop() {
    unsafe {
        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn main_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_HOTKEY => {
                let id = wparam.0 as i32;
                debug_log(&format!("hotkey id={id}"));
                #[cfg(debug_assertions)]
                if id == HOTKEY_CLIPBOARD_TEST_ID {
                    spawn_clipboard_test();
                    return LRESULT(0);
                }
                let action = if id == HOTKEY_ASK_AI_ID {
                    ACTION_ASK_QUICK
                } else {
                    ACTION_TRANSLATE
                };
                spawn_hotkey_capture(action);
                LRESULT(0)
            }
            tray::TRAY_CALLBACK_MESSAGE => {
                if tray::is_right_click(lparam.0) {
                    show_tray_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wparam.0 & 0xFFFF;
                if id >= MENU_PROVIDER_BASE {
                    switch_provider(id - MENU_PROVIDER_BASE);
                } else {
                    match id {
                        MENU_TEST_CAPTURE => spawn_hotkey_capture(ACTION_TRANSLATE),
                        MENU_EXIT => {
                            let _ = destroy_window_safe(hwnd);
                        }
                        MENU_SET_KEY => prompt_api_key(hwnd),
                        MENU_AUTOSTART => toggle_autostart(),
                        MENU_AUTO_FALLBACK => toggle_auto_fallback(),
                        MENU_PROVIDER_ADD => add_provider(hwnd),
                        MENU_PROVIDER_EDIT => edit_active_provider(hwnd),
                        MENU_PROVIDER_DELETE => delete_active_provider(hwnd),
                        MENU_FETCH_MODELS => fetch_models(),
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            WM_XUANCI_MOUSE => {
                handle_mouse(wparam, lparam);
                LRESULT(0)
            }
            WM_XUANCI_FOREGROUND => {
                let tb = hwnd_from(TOOLBAR_HWND.load(Ordering::Relaxed));
                if !tb.0.is_null() && toolbar::is_visible(tb) {
                    toolbar::hide(tb);
                }
                LRESULT(0)
            }
            WM_XUANCI_CAPTURED => {
                show_toolbar();
                LRESULT(0)
            }
            WM_XUANCI_CAPTURE_FAILED => LRESULT(0),
            WM_XUANCI_ACTION => {
                run_action(wparam.0);
                LRESULT(0)
            }
            WM_XUANCI_MODELS_READY => {
                handle_models_ready();
                LRESULT(0)
            }
            WM_DESTROY => {
                let tb = hwnd_from(TOOLBAR_HWND.load(Ordering::Relaxed));
                if !tb.0.is_null() {
                    toolbar::destroy(tb);
                }
                tray::remove(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => window::default_proc(hwnd, msg, wparam, lparam),
        }
    }
}

fn handle_mouse(wparam: WPARAM, lparam: LPARAM) {
    let event = wparam.0;
    let (x, y) = messages::unpack_point(lparam.0);
    let tb = hwnd_from(TOOLBAR_HWND.load(Ordering::Relaxed));
    let visible = !tb.0.is_null() && toolbar::is_visible(tb);

    match event {
        MOUSE_DOWN => {
            if visible && !toolbar::contains_point(tb, x, y) {
                toolbar::hide(tb);
            }
            if let Ok(mut slot) = MOUSE_DOWN_AT.lock() {
                *slot = Some((x, y, Instant::now()));
            }
        }
        MOUSE_UP => {
            let down = MOUSE_DOWN_AT.lock().ok().and_then(|mut slot| slot.take());
            let Some((down_x, down_y, down_at)) = down else {
                return;
            };
            let dx = x - down_x;
            let dy = y - down_y;
            let dragged = dx * dx + dy * dy >= DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX
                && down_at.elapsed().as_millis() >= DRAG_MIN_MS;

            if visible && toolbar::contains_point(tb, x, y) {
                return;
            }

            if dragged {
                if let Ok(mut slot) = LAST_CLICK.lock() {
                    *slot = None;
                }
                spawn_auto_capture();
                return;
            }

            let previous = LAST_CLICK.lock().ok().and_then(|mut slot| slot.take());
            let is_double_click = previous.map_or(false, |(px, py, at)| {
                let ddx = x - px;
                let ddy = y - py;
                ddx * ddx + ddy * ddy <= DOUBLE_CLICK_SLOP_PX * DOUBLE_CLICK_SLOP_PX
                    && at.elapsed().as_millis() <= DOUBLE_CLICK_MS
            });

            if is_double_click {
                spawn_auto_capture_delayed(160);
            } else if let Ok(mut slot) = LAST_CLICK.lock() {
                *slot = Some((x, y, Instant::now()));
            }
        }
        MOUSE_WHEEL | MOUSE_OTHER_BUTTON => {
            if visible {
                toolbar::hide(tb);
            }
        }
        _ => {}
    }
}

fn show_toolbar() {
    let cursor = LAST_SELECTION
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|c| (c.cursor_x, c.cursor_y)));
    let Some((x, y)) = cursor else {
        return;
    };
    let tb = hwnd_from(TOOLBAR_HWND.load(Ordering::Relaxed));
    if tb.0.is_null() {
        return;
    }
    if let Err(e) = toolbar::show_near_cursor(tb, x, y) {
        debug_log(&format!("toolbar show failed: {e}"));
    }
}

fn run_action(index: usize) {
    let Some(ctx) = take_selection() else {
        debug_log(&format!("action {index}: no selection"));
        return;
    };
    debug_log(&format!(
        "action {index}: chars={} provider={} key={}",
        ctx.text.chars().count(),
        settings_snapshot().active_name(),
        active_api_key().is_some()
    ));

    match index {
        ACTION_TRANSLATE => start_translate(ctx),
        ACTION_ASK_AI => {
            let spec = InputSpec {
                title: "AI 问答",
                label: "输入你的问题（留空则解释选中内容）",
                initial: "",
                password: false,
                multiline: false,
            };
            if let Some(question) = dialog::input_text(app_hwnd(), spec) {
                start_ask(ctx, question);
            }
        }
        ACTION_ASK_QUICK => start_ask(ctx, String::new()),
        index if index >= 2 => {
            let templates = template_snapshot();
            if let Some(template) = templates.get(index - 2).cloned() {
                start_template(ctx, template);
            }
        }
        _ => {}
    }
}

fn start_translate(ctx: SelectionContext) {
    let Some(api_key) = active_api_key() else {
        prompt_key_or_info();
        return;
    };
    let settings = settings_snapshot();
    let request = prompt::translate_request(&settings, &ctx.text);
    launch_result("翻译", settings, api_key, request);
}

fn start_ask(ctx: SelectionContext, question: String) {
    let Some(api_key) = active_api_key() else {
        prompt_key_or_info();
        return;
    };
    let settings = settings_snapshot();
    let request = prompt::ask_request(&settings, &ctx.text, &question);
    launch_result("AI 问答", settings, api_key, request);
}

fn start_template(ctx: SelectionContext, template: PromptTemplate) {
    let Some(api_key) = active_api_key() else {
        prompt_key_or_info();
        return;
    };
    let settings = settings_snapshot();
    let request = template.to_request(&settings, &ctx.text);
    launch_result(&template.name, settings, api_key, request);
}

fn prompt_key_or_info() {
    let name = settings_snapshot().active_name();
    dialog::info(&format!(
        "当前服务商「{name}」还没有 API Key。\n请在托盘菜单中设置当前 API Key。"
    ));
}

fn launch_result(title: &str, settings: AppSettings, api_key: String, request: ChatRequest) {
    debug_log(&format!(
        "launch_result provider={} endpoint={}",
        settings.active_name(),
        settings.endpoint()
    ));
    match result::open(app_hwnd(), title) {
        Ok(hwnd) => {
            debug_log("result window opened");
            match result::start_stream(hwnd, title, settings, api_key, request) {
                Ok(()) => debug_log("stream started"),
                Err(e) => {
                    debug_log(&format!("start_stream failed: {e}"));
                    dialog::error(&format!("启动请求失败：{e}"));
                }
            }
        }
        Err(e) => {
            debug_log(&format!("result open failed: {e}"));
            dialog::error(&format!("打开结果窗口失败：{e}"));
        }
    }
}

fn spawn_auto_capture() {
    spawn_auto_capture_delayed(0);
}

fn spawn_auto_capture_delayed(delay_ms: u64) {
    let allow_fallback = settings_snapshot().clipboard_fallback_auto;
    std::thread::Builder::new()
        .name("xuanci-auto-capture".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            if delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
            let started = std::time::Instant::now();
            let outcome = pipeline::capture(TriggerMode::AutoPopup, allow_fallback);
            let elapsed = started.elapsed().as_millis();
            match outcome {
                Ok(ctx) => {
                    debug_log(&format!(
                        "auto capture ok chars={} method={:?} elapsed_ms={elapsed}",
                        ctx.text.chars().count(),
                        ctx.capture_method
                    ));
                    if let Ok(mut slot) = LAST_SELECTION.lock() {
                        *slot = Some(ctx);
                    }
                    post_to_app(WM_XUANCI_CAPTURED);
                }
                Err(e) => {
                    debug_log(&format!("auto capture failed elapsed_ms={elapsed}: {e}"));
                    post_to_app(WM_XUANCI_CAPTURE_FAILED);
                }
            }
        })
        .ok();
}

fn spawn_hotkey_capture(action: usize) {
    std::thread::Builder::new()
        .name("xuanci-hotkey-capture".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let started = std::time::Instant::now();
            match pipeline::capture(TriggerMode::Hotkey, true) {
                Ok(ctx) => {
                    debug_log(&format!(
                        "hotkey capture ok chars={} method={:?} elapsed_ms={}",
                        ctx.text.chars().count(),
                        ctx.capture_method,
                        started.elapsed().as_millis()
                    ));
                    if let Ok(mut slot) = LAST_SELECTION.lock() {
                        *slot = Some(ctx);
                    }
                    post_to_app_action(action);
                }
                Err(e) => {
                    debug_log(&format!("hotkey capture failed: {e}"));
                    dialog::info(&format!("取词失败：{e}"));
                }
            }
        })
        .ok();
}

fn post_to_app(msg: u32) {
    let app = app_hwnd();
    if !app.0.is_null() {
        unsafe {
            let _ = PostMessageW(Some(app), msg, WPARAM(0), LPARAM(0));
        }
    }
}

fn post_to_app_action(index: usize) {
    let app = app_hwnd();
    if !app.0.is_null() {
        unsafe {
            let _ = PostMessageW(Some(app), WM_XUANCI_ACTION, WPARAM(index), LPARAM(0));
        }
    }
}

fn prompt_api_key(parent: HWND) {
    let settings = settings_snapshot();
    let name = settings.active_name();
    let label = format!("{name} 的 API Key（DPAPI 加密保存在本机）");
    let spec = InputSpec {
        title: "XuanCi 设置",
        label: &label,
        initial: "",
        password: true,
        multiline: false,
    };
    let Some(key) = dialog::input_text(parent, spec) else {
        return;
    };
    let key = key.trim().to_string();
    if key.is_empty() {
        return;
    }
    match set_key_for(&settings.active_provider, &key) {
        Ok(()) => dialog::info(&format!("已保存「{name}」的 API Key。")),
        Err(e) => dialog::error(&format!("保存 API Key 失败：{e}")),
    }
}

fn switch_provider(index: usize) {
    let mut settings = settings_snapshot();
    let Some(profile) = settings.providers.get(index).cloned() else {
        return;
    };
    if !settings.switch_provider(&profile.id) {
        return;
    }
    if let Err(e) = save_settings_cell(&settings) {
        dialog::error(&format!("保存设置失败：{e}"));
        return;
    }
    if key_for(&profile.id).is_none() {
        dialog::info(&format!(
            "已切换到「{}」。\n该服务商还没有保存 API Key，请通过「设置当前 API Key…」配置。",
            profile.name
        ));
    }
}

fn add_provider(parent: HWND) {
    let spec = InputSpec {
        title: "添加服务商",
        label: "Base URL（例如 https://api.openai.com/v1）",
        initial: "https://api.openai.com/v1",
        password: false,
        multiline: false,
    };
    let Some(base) = dialog::input_text(parent, spec) else {
        return;
    };
    let base = base.trim().trim_end_matches('/').to_string();
    if base.is_empty() || !base_url_allowed(&base) {
        dialog::error("Base URL 必须以 https:// 开头（本地 127.0.0.1/localhost 除外）。");
        return;
    }

    let (id_hint, name_hint, model_hint) = detect_provider(&base);

    let model_initial = model_hint.to_string();
    let spec = InputSpec {
        title: "添加服务商",
        label: "模型名（可稍后用「获取模型列表」修改）",
        initial: &model_initial,
        password: false,
        multiline: false,
    };
    let Some(model) = dialog::input_text(parent, spec) else {
        return;
    };
    let model = model.trim().to_string();
    if model.is_empty() {
        dialog::error("模型名不能为空。");
        return;
    }

    let name_initial = name_hint.to_string();
    let spec = InputSpec {
        title: "添加服务商",
        label: "显示名称",
        initial: &name_initial,
        password: false,
        multiline: false,
    };
    let Some(name) = dialog::input_text(parent, spec) else {
        return;
    };
    let name = name.trim().to_string();
    let name = if name.is_empty() {
        name_hint.to_string()
    } else {
        name
    };

    let spec = InputSpec {
        title: "添加服务商",
        label: "API Key（本地模型可填 local）",
        initial: "",
        password: true,
        multiline: false,
    };
    let Some(key) = dialog::input_text(parent, spec) else {
        return;
    };
    let mut key = key.trim().to_string();
    if key.is_empty() {
        if base.contains("127.0.0.1") || base.contains("localhost") {
            key = "local".to_string();
        } else {
            dialog::error("API Key 不能为空。");
            return;
        }
    }

    let settings = settings_snapshot();
    let mut id = id_hint.to_string();
    let mut suffix = 2;
    while settings.providers.iter().any(|provider| provider.id == id) {
        id = format!("{id_hint}-{suffix}");
        suffix += 1;
    }

    let profile = ProviderProfile {
        id: id.clone(),
        name: name.clone(),
        base_url: base,
        model,
    };

    let mut settings = settings;
    settings.upsert_provider(profile);
    settings.active_provider = id.clone();
    settings.sync_active();
    if let Err(e) = save_settings_cell(&settings) {
        dialog::error(&format!("保存设置失败：{e}"));
        return;
    }
    if let Err(e) = set_key_for(&id, &key) {
        dialog::error(&format!("添加成功，但保存 API Key 失败：{e}"));
        return;
    }
    dialog::info(&format!("已添加并切换到服务商「{name}」。"));
}

fn edit_active_provider(parent: HWND) {
    let settings = settings_snapshot();
    let Some(profile) = settings.active().cloned() else {
        return;
    };

    let spec = InputSpec {
        title: "编辑服务商",
        label: "显示名称",
        initial: &profile.name,
        password: false,
        multiline: false,
    };
    let Some(name) = dialog::input_text(parent, spec) else {
        return;
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        dialog::error("显示名称不能为空。");
        return;
    }

    let spec = InputSpec {
        title: "编辑服务商",
        label: "Base URL",
        initial: &profile.base_url,
        password: false,
        multiline: false,
    };
    let Some(base) = dialog::input_text(parent, spec) else {
        return;
    };
    let base = base.trim().trim_end_matches('/').to_string();
    if !base_url_allowed(&base) {
        dialog::error("Base URL 必须以 https:// 开头（本地地址除外）。");
        return;
    }

    let spec = InputSpec {
        title: "编辑服务商",
        label: "模型名",
        initial: &profile.model,
        password: false,
        multiline: false,
    };
    let Some(model) = dialog::input_text(parent, spec) else {
        return;
    };
    let model = model.trim().to_string();
    if model.is_empty() {
        dialog::error("模型名不能为空。");
        return;
    }

    let mut settings = settings;
    settings.upsert_provider(ProviderProfile {
        id: profile.id.clone(),
        name,
        base_url: base,
        model,
    });
    settings.active_provider = profile.id;
    settings.sync_active();
    if let Err(e) = save_settings_cell(&settings) {
        dialog::error(&format!("保存设置失败：{e}"));
    }
}

fn delete_active_provider(parent: HWND) {
    let _ = parent;
    let settings = settings_snapshot();
    let Some(profile) = settings.active().cloned() else {
        return;
    };
    if !dialog::confirm(&format!(
        "确定删除服务商「{}」吗？\n它的 API Key 也会被删除。",
        profile.name
    )) {
        return;
    }

    let mut settings = settings;
    if !settings.remove_provider(&profile.id) {
        dialog::error("至少需要保留一个服务商。");
        return;
    }
    if let Err(e) = save_settings_cell(&settings) {
        dialog::error(&format!("保存设置失败：{e}"));
        return;
    }
    remove_key_for(&profile.id);
    dialog::info(&format!("已删除服务商「{}」。", profile.name));
}

fn fetch_models() {
    let settings = settings_snapshot();
    let Some(api_key) = key_for(&settings.active_provider) else {
        prompt_key_or_info();
        return;
    };
    let provider_id = settings.active_provider.clone();
    std::thread::Builder::new()
        .name("xuanci-models".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let result = xuanci_core::deepseek::list_models(&settings, &api_key);
            if let Ok(mut slot) = MODELS_RESULT.lock() {
                *slot = Some((provider_id, result));
            }
            post_to_app(WM_XUANCI_MODELS_READY);
        })
        .ok();
}

fn handle_models_ready() {
    let result = MODELS_RESULT.lock().ok().and_then(|mut slot| slot.take());
    let Some((provider_id, result)) = result else {
        return;
    };
    match result {
        Ok(models) => {
            let settings = settings_snapshot();
            let name = settings
                .providers
                .iter()
                .find(|provider| provider.id == provider_id)
                .map(|provider| provider.name.clone())
                .unwrap_or_else(|| "当前服务商".to_string());
            let label = format!("{name} 可用的模型（共 {} 个）", models.len());
            let Some(model) = dialog::select_item(app_hwnd(), "选择模型", &label, &models) else {
                return;
            };
            let mut settings = settings;
            if let Some(provider) = settings
                .providers
                .iter_mut()
                .find(|provider| provider.id == provider_id)
            {
                provider.model = model.clone();
            }
            if settings.active_provider == provider_id {
                settings.sync_active();
            }
            match save_settings_cell(&settings) {
                Ok(()) => dialog::info(&format!("模型已设置为：{model}")),
                Err(e) => dialog::error(&format!("保存设置失败：{e}")),
            }
        }
        Err(e) => dialog::error(&format!("获取模型列表失败：{e}")),
    }
}

fn toggle_auto_fallback() {
    let mut settings = settings_snapshot();
    settings.clipboard_fallback_auto = !settings.clipboard_fallback_auto;
    if let Err(e) = save_settings_cell(&settings) {
        dialog::error(&format!("保存设置失败：{e}"));
    }
}

fn toggle_autostart() {
    let enable = !autostart::is_enabled();
    match autostart::set_enabled(enable) {
        Ok(()) => {
            if enable {
                dialog::info("已开启开机自启。\n下次登录 Windows 时会自动运行 XuanCi。");
            } else {
                dialog::info("已关闭开机自启。");
            }
        }
        Err(e) => dialog::error(&format!("设置开机自启失败：{e}")),
    }
}

unsafe fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let settings = settings_snapshot();
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let Ok(submenu) = CreatePopupMenu() else {
            let _ = DestroyMenu(menu);
            return;
        };

        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TEST_CAPTURE,
            w!("取词并翻译\tCtrl+Alt+D"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        for (index, provider) in settings.providers.iter().enumerate() {
            let flags = if provider.id == settings.active_provider {
                MENU_ITEM_FLAGS(MF_STRING.0 | MF_CHECKED.0)
            } else {
                MF_STRING
            };
            let label = HSTRING::from(format!("{} · {}", provider.name, provider.model));
            let _ = AppendMenuW(
                submenu,
                flags,
                MENU_PROVIDER_BASE + index,
                PCWSTR(label.as_ptr()),
            );
        }
        let _ = AppendMenuW(submenu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(submenu, MF_STRING, MENU_PROVIDER_ADD, w!("添加服务商…"));
        let _ = AppendMenuW(submenu, MF_STRING, MENU_PROVIDER_EDIT, w!("编辑当前服务商…"));
        let _ = AppendMenuW(submenu, MF_STRING, MENU_PROVIDER_DELETE, w!("删除当前服务商"));

        let sub_label = HSTRING::from(format!("服务商：{}", settings.active_name()));
        let _ = AppendMenuW(
            menu,
            MF_POPUP,
            submenu.0 as usize,
            PCWSTR(sub_label.as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_FETCH_MODELS, w!("获取模型列表…"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_SET_KEY, w!("设置当前 API Key…"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        let autostart_flags = if autostart::is_enabled() {
            MENU_ITEM_FLAGS(MF_STRING.0 | MF_CHECKED.0)
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(menu, autostart_flags, MENU_AUTOSTART, w!("开机自启"));

        let fallback_flags = if settings.clipboard_fallback_auto {
            MENU_ITEM_FLAGS(MF_STRING.0 | MF_CHECKED.0)
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(
            menu,
            fallback_flags,
            MENU_AUTO_FALLBACK,
            w!("自动弹条使用剪贴板回退"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, w!("退出"));

        let mut pt = Default::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(submenu);
        let _ = DestroyMenu(menu);
    }
}

#[cfg(debug_assertions)]
fn spawn_clipboard_test() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let started = std::time::Instant::now();
            let result = (|| {
                let info = xuanci_win::snapshot::foreground_target()
                    .ok_or_else(|| "no foreground".to_string())?;
                xuanci_win::clipboard::capture_via_ctrl_c(info.hwnd)
            })();
            match &result {
                Ok(text) => debug_log(&format!(
                    "clipboard ok chars={} elapsed_ms={}",
                    text.chars().count(),
                    started.elapsed().as_millis()
                )),
                Err(e) => debug_log(&format!(
                    "clipboard failed elapsed_ms={}: {e}",
                    started.elapsed().as_millis()
                )),
            }
        })
        .ok();
}

/// Debug-only log: records outcome metadata, never selected text.
fn debug_log(msg: &str) {
    if !cfg!(debug_assertions) {
        return;
    }
    use std::io::Write;
    let path = std::env::temp_dir().join("xuanci-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

unsafe fn destroy_window_safe(hwnd: HWND) -> windows::core::Result<()> {
    unsafe { windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd) }
}
