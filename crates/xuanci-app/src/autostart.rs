//! Per-user autostart via HKCU\...\Run.

use windows::core::w;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

const RUN_SUBKEY: windows::core::PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: windows::core::PCWSTR = w!("XuanCi");

fn open_run_key(extra_access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Result<HKEY, String> {
    unsafe {
        let mut key = HKEY::default();
        let status = RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, Some(0), extra_access, &mut key);
        if status != ERROR_SUCCESS {
            return Err(format!("无法打开注册表 Run 项（错误码 {}）", status.0));
        }
        Ok(key)
    }
}

fn command_line() -> String {
    let exe = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    format!("\"{exe}\" --autostart")
}

pub fn is_enabled() -> bool {
    unsafe {
        let Ok(key) = open_run_key(KEY_QUERY_VALUE) else {
            return false;
        };

        let mut size = 0u32;
        let status = RegQueryValueExW(key, VALUE_NAME, None, None, None, Some(&mut size));
        if status != ERROR_SUCCESS || size < 2 {
            let _ = RegCloseKey(key);
            return false;
        }

        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let mut read = size;
        let status = RegQueryValueExW(
            key,
            VALUE_NAME,
            None,
            None,
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut read),
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return false;
        }

        let len = buf
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(buf.len());
        let stored = String::from_utf16_lossy(&buf[..len]);
        stored.to_lowercase() == command_line().to_lowercase()
    }
}

pub fn set_enabled(enable: bool) -> Result<(), String> {
    unsafe {
        let key = open_run_key(KEY_SET_VALUE)?;

        if enable {
            let command = command_line();
            let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(
                wide.as_ptr() as *const u8,
                wide.len() * std::mem::size_of::<u16>(),
            );
            let status = RegSetValueExW(key, VALUE_NAME, Some(0), REG_SZ, Some(bytes));
            let _ = RegCloseKey(key);
            if status != ERROR_SUCCESS {
                return Err(format!("写入启动项失败（错误码 {}）", status.0));
            }
        } else {
            let status = RegDeleteValueW(key, VALUE_NAME);
            let _ = RegCloseKey(key);
            if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
                return Err(format!("删除启动项失败（错误码 {}）", status.0));
            }
        }
    }
    Ok(())
}
