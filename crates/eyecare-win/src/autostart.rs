//! 开机自启：写当前用户的注册表 Run 键。
//!
//! 为什么不用计划任务、也不用启动文件夹：
//! - Run 键最简单，不需要管理员权限，用户在任务管理器的「启动」里能看到并禁用
//! - 启动文件夹要建快捷方式，得走 COM，麻烦且难清理
//!
//! 只写 HKEY_CURRENT_USER，不碰 HKLM，所以永远不需要提权。

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn open_run_key(access: u32) -> Option<HKEY> {
    let sub = to_wide(RUN_KEY);
    let mut key: HKEY = null_mut();
    let rc = RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, access, &mut key);
    if rc == ERROR_SUCCESS {
        Some(key)
    } else {
        None
    }
}

/// 这个名字在注册表里是否已经有值。
pub fn is_enabled(app_name: &str) -> bool {
    unsafe {
        let Some(key) = open_run_key(KEY_QUERY_VALUE) else {
            return false;
        };
        let name = to_wide(app_name);
        let mut kind: u32 = 0;
        let mut size: u32 = 0;
        let rc = RegQueryValueExW(key, name.as_ptr(), null(), &mut kind, null_mut(), &mut size);
        RegCloseKey(key);
        rc == ERROR_SUCCESS && size > 2
    }
}

/// 读出当前登记的命令行，方便界面展示和排查。
pub fn current_command(app_name: &str) -> Option<String> {
    unsafe {
        let key = open_run_key(KEY_QUERY_VALUE)?;
        let name = to_wide(app_name);

        let mut size: u32 = 0;
        if RegQueryValueExW(key, name.as_ptr(), null(), null_mut(), null_mut(), &mut size)
            != ERROR_SUCCESS
            || size == 0
        {
            RegCloseKey(key);
            return None;
        }

        let mut buf = vec![0u16; (size as usize / 2) + 1];
        let mut kind: u32 = 0;
        let rc = RegQueryValueExW(
            key,
            name.as_ptr(),
            null(),
            &mut kind,
            buf.as_mut_ptr() as *mut u8,
            &mut size,
        );
        RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            return None;
        }

        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

/// 写入或删除。`command` 传 None 表示关闭自启。
/// 返回 false 表示注册表操作失败，调用方应该据此把界面开关拨回去。
pub fn set_enabled(app_name: &str, command: Option<&str>) -> bool {
    unsafe {
        let Some(key) = open_run_key(KEY_SET_VALUE) else {
            return false;
        };
        let name = to_wide(app_name);

        let ok = match command {
            Some(cmd) => {
                let data = to_wide(cmd);
                RegSetValueExW(
                    key,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    data.as_ptr() as *const c_void as *const u8,
                    (data.len() * 2) as u32,
                ) == ERROR_SUCCESS
            }
            None => {
                let rc = RegDeleteValueW(key, name.as_ptr());
                rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND
            }
        };

        RegCloseKey(key);
        ok
    }
}
