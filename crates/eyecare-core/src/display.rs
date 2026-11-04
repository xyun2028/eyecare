//! 显示器抽象层。
//!
//! 上层只认这个 trait，不关心底下是 Windows、macOS 还是 Linux。
//! 以后加平台只需要多一个实现，调度和界面代码一行都不用改。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// 一块屏幕的静态信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayInfo {
    /// 系统内部标识，Windows 下形如 `\\.\DISPLAY1`
    pub id: String,
    /// 显示器的对外型号名，取不到就和 id 相同
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    /// `None` 表示这台机器查不了或查不到，不能当成「没开 HDR」
    pub hdr: Option<bool>,
}

impl DisplayInfo {
    /// 给人看的一行描述，CLI 和设置界面共用。
    pub fn label(&self) -> String {
        let mut s = format!("{} · {}x{}", self.name, self.width, self.height);
        if self.primary {
            s.push_str(" · 主屏");
        }
        match self.hdr {
            Some(true) => s.push_str(" · HDR 已开启"),
            Some(false) => s.push_str(" · SDR"),
            None => {}
        }
        s
    }
}

/// 平台后端需要实现的能力。
pub trait DisplayBackend {
    fn list(&self) -> Vec<DisplayInfo>;
    /// 读取某块屏当前的 Gamma 表，用于备份和校验。
    fn read_ramp(&self, id: &str) -> Option<[u16; 768]>;
    /// 把 Gamma 表写到某块屏。返回值只代表系统是否接受，不代表肉眼生效。
    fn write_ramp(&self, id: &str, ramp: &[u16; 768]) -> bool;
}

/// 备份当前所有屏幕的 Gamma 表。
pub fn snapshot(backend: &dyn DisplayBackend) -> Vec<(String, [u16; 768])> {
    backend
        .list()
        .into_iter()
        .filter_map(|d| backend.read_ramp(&d.id).map(|r| (d.id, r)))
        .collect()
}

/// 把备份原样写回去。
pub fn restore_all(backend: &dyn DisplayBackend, saved: &[(String, [u16; 768])]) {
    for (id, ramp) in saved {
        backend.write_ramp(id, ramp);
    }
}

/// 状态文件的位置：优先放在用户本地数据目录，退出登录也不会丢。
pub fn default_state_path() -> PathBuf {
    if let Ok(base) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(base).join("eyecare").join("last_ramp.txt");
    }
    std::env::temp_dir().join("eyecare").join("last_ramp.txt")
}

/// 在改动屏幕之前，先把原始表落盘。
///
/// 这是对付「进程被强杀」的唯一手段：程序来不及做清理，
/// 但下次启动时可以靠这个文件把屏幕救回来。
pub fn save_state(path: &Path, saved: &[(String, [u16; 768])]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut text = String::from("eyecare-state-v1\n");
    for (id, ramp) in saved {
        text.push_str(id);
        for v in ramp {
            text.push(',');
            text.push_str(&v.to_string());
        }
        text.push('\n');
    }
    fs::write(path, text)
}

/// 读回上次落盘的原始表。文件不存在或格式不对都返回空。
pub fn load_state(path: &Path) -> Vec<(String, [u16; 768])> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let mut parts = line.split(',');
        let Some(id) = parts.next() else { continue };
        if id.is_empty() {
            continue;
        }
        let mut ramp = [0u16; 768];
        let mut count = 0;
        for (i, p) in parts.enumerate().take(768) {
            match p.parse::<u16>() {
                Ok(v) => {
                    ramp[i] = v;
                    count += 1;
                }
                Err(_) => break,
            }
        }
        if count == 768 {
            out.push((id.to_string(), ramp));
        }
    }
    out
}

pub fn clear_state(path: &Path) {
    let _ = fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 状态文件能原样读写() {
        let dir = std::env::temp_dir().join("eyecare-test-state");
        let path = dir.join("ramp.txt");
        let _ = fs::remove_dir_all(&dir);

        let saved = vec![
            (r"\\.\DISPLAY1".to_string(), crate::identity_ramp()),
            (r"\\.\DISPLAY2".to_string(), crate::build_gamma_ramp(2700, 1.0)),
        ];
        save_state(&path, &saved).unwrap();
        let loaded = load_state(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].0, r"\\.\DISPLAY1");
        assert_eq!(loaded[0].1, saved[0].1);
        assert_eq!(loaded[1].1, saved[1].1);

        clear_state(&path);
        assert!(load_state(&path).is_empty());
    }

    #[test]
    fn 状态文件损坏时不崩溃() {
        let path = std::env::temp_dir().join("eyecare-test-broken.txt");
        fs::write(&path, "garbage\n\\.\\DISPLAY1,1,2,3\n").unwrap();
        assert!(load_state(&path).is_empty());
        let _ = fs::remove_file(&path);
    }
}
