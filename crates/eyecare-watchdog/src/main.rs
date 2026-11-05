//! 看门狗：主程序异常死亡时，把屏幕颜色还原。
//!
//! 为什么需要它：主程序被任务管理器结束、被 `taskkill /F`、或者直接崩溃时，
//! 没有任何机会执行清理代码，屏幕会一直停在偏色状态。用户不重新打开程序就恢复不了。
//!
//! 做法很土但有效：主程序启动时把自己之前落盘的「原始 Gamma 表」交给这里，
//! 看门狗只等主进程退出。退出后发现备份文件**还在**，说明不是干净退出，直接写回。
//! 主程序正常退出会先删掉备份文件，看门狗看到文件没了就什么都不做，安静退出。

#![windows_subsystem = "windows"]

use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use eyecare_core::display::{clear_state, load_state, restore_all};
use eyecare_win::WindowsBackend;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess};

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const STILL_ACTIVE: u32 = 259;

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let pos = args.iter().position(|a| a == key)?;
    args.get(pos + 1).cloned()
}

/// 主进程是否还活着。用退出码判断，比只看句柄能不能打开更准。
fn process_alive(pid: u32) -> bool {
    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        // 读不到就当它已经没了，宁可多恢复一次
        ok != 0 && code == STILL_ACTIVE
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(parent) = arg_value(&args, "--parent").and_then(|s| s.parse::<u32>().ok()) else {
        return;
    };
    let Some(state) = arg_value(&args, "--state") else {
        return;
    };

    // 盯着主进程，直到它消失
    while process_alive(parent) {
        sleep(Duration::from_millis(700));
    }

    // 给它一点余量，避免主进程正在收尾时抢先动手
    sleep(Duration::from_millis(600));

    let path = Path::new(&state);
    let saved = load_state(path);
    if saved.is_empty() {
        // 干净退出，主程序已经删掉备份并还原过屏幕
        return;
    }

    let backend = WindowsBackend::new();
    restore_all(&backend, &saved);
    clear_state(path);
}
