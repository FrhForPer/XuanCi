//! Optional LaTeX formula plugin management. The plugin is never bundled with
//! XuanCi and is not installed unless the user explicitly installs it.

use std::fs;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;

use xuanci_win::{dialog, paths};

const PLUGIN_EXE: &str = "xuanci-latex.exe";

pub fn plugin_dir() -> PathBuf {
    paths::app_data_dir().join("plugins").join("latex")
}

pub fn plugin_exe() -> PathBuf {
    plugin_dir().join(PLUGIN_EXE)
}

pub fn installed() -> bool {
    plugin_exe().is_file()
}

pub fn plugin_path(enabled: bool) -> Option<PathBuf> {
    if !enabled {
        return None;
    }
    let path = plugin_exe();
    if xuanci_core::latex_plugin::probe(&path) {
        Some(path)
    } else {
        None
    }
}

pub fn status() -> String {
    let path = plugin_exe();
    if !path.is_file() {
        return "未安装（默认）".to_string();
    }
    match xuanci_core::latex_plugin::version(&path) {
        Some(version) => format!("已安装：{version}"),
        None => "已安装，但插件无法运行".to_string(),
    }
}

pub fn install(parent: HWND) {
    let Some(source) = dialog::open_file(parent, "选择 xuanci-latex.exe 插件") else {
        return;
    };
    if !xuanci_core::latex_plugin::probe(&source) {
        dialog::error(
            "这不是有效的 XuanCi LaTeX 插件。\n请选择官方发布的 xuanci-latex.exe。",
        );
        return;
    }

    let dir = plugin_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        dialog::error(&format!("创建插件目录失败：{e}"));
        return;
    }
    let target = plugin_exe();
    if source != target {
        if let Err(e) = fs::copy(&source, &target) {
            dialog::error(&format!("复制插件失败：{e}"));
            return;
        }
    }
    if !xuanci_core::latex_plugin::probe(&target) {
        let _ = fs::remove_file(&target);
        dialog::error("插件复制后无法运行，已回滚。");
        return;
    }

    let version = xuanci_core::latex_plugin::version(&target).unwrap_or_default();
    let manifest = serde_json::json!({
        "id": "latex",
        "name": "LaTeX 公式",
        "version": version,
        "executable": PLUGIN_EXE,
        "protocol": 1
    });
    let _ = fs::write(
        dir.join("plugin.json"),
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    );
    dialog::info("LaTeX 插件已安装，将在下次渲染 Markdown 时生效。");
}

pub fn uninstall(_parent: HWND) {
    if !installed() {
        dialog::info("LaTeX 插件尚未安装。");
        return;
    }
    if !dialog::confirm("确定卸载 LaTeX 插件吗？\n公式将回退为 LaTeX 源码显示。") {
        return;
    }
    let dir = plugin_dir();
    match fs::remove_dir_all(&dir) {
        Ok(()) => dialog::info("LaTeX 插件已卸载。"),
        Err(e) => dialog::error(&format!("卸载失败：{e}")),
    }
}

pub fn open_dir() {
    let dir = plugin_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer")
        .arg(Path::new(&dir))
        .spawn();
}
