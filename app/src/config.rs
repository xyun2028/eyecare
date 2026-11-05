//! 配置结构、预设清单、以及「现在该用什么颜色」的换算。
//!
//! 这一层不做任何系统调用，纯粹是数据，方便单测。

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const PRESET_CUSTOM: &str = "custom";

/// 一天中的分钟数，带小数，用来做平滑过渡。
pub fn minutes_now() -> f64 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    now.hour() as f64 * 60.0 + now.minute() as f64 + now.second() as f64 / 60.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub enabled: bool,
    pub day_kelvin: u32,
    pub night_kelvin: u32,
    /// 白天开始时刻，距零点的分钟数
    pub day_start: u32,
    /// 夜间开始时刻
    pub night_start: u32,
    /// 切换的过渡时长，单位分钟。到点突变眼睛会不适应，这里做线性过渡
    pub transition: u32,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            day_kelvin: 5500,
            night_kelvin: 3000,
            day_start: 7 * 60,
            night_start: 19 * 60,
            transition: 60,
        }
    }
}

fn lerp(from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * t.clamp(0.0, 1.0)
}

impl Schedule {
    /// 给定一天中的分钟数，算出当时该用的色温。
    pub fn kelvin_at(&self, minutes: f64) -> u32 {
        let day_k = self.day_kelvin as f64;
        let night_k = self.night_kelvin as f64;
        let tr = self.transition.max(1) as f64;
        let day_s = self.day_start as f64;
        let night_s = self.night_start as f64;

        let up_end = day_s + tr;
        let down_start = night_s - tr;

        let value = if minutes >= up_end && minutes < down_start {
            day_k
        } else if minutes >= day_s && minutes < up_end {
            lerp(night_k, day_k, (minutes - day_s) / tr)
        } else if minutes >= down_start && minutes < night_s {
            lerp(day_k, night_k, (minutes - down_start) / tr)
        } else {
            night_k
        };

        value.round().clamp(1000.0, 6500.0) as u32
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    /// 预设 id，或 "custom"
    pub preset: String,
    pub kelvin: u32,
    pub brightness: f64,
    /// 自定义取色器选的颜色
    pub color: Option<[u8; 3]>,
    /// "all" 或具体显示器 id
    pub target: String,
    /// 界面主题："system" / "light" / "dark"
    #[serde(default = "default_theme")]
    pub theme: String,
    pub schedule: Schedule,
}

fn default_theme() -> String {
    "system".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            preset: "office".to_string(),
            kelvin: 4500,
            brightness: 1.0,
            color: None,
            target: "all".to_string(),
            theme: default_theme(),
            schedule: Schedule::default(),
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("eyecare").join("config.json")
}

impl Config {
    pub fn load() -> Self {
        fs::read_to_string(config_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, text);
        }
    }
}

/// 界面上的一张预设卡片。
#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub hint: &'static str,
    pub kelvin: Option<u32>,
    pub color: Option<[u8; 3]>,
    /// 卡片上用来示意效果的颜色
    pub swatch: &'static str,
}

pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            id: "off",
            name: "原色",
            hint: "不做任何调整",
            kelvin: Some(6500),
            color: None,
            swatch: "#eef1f5",
        },
        Preset {
            id: "soft",
            name: "轻度",
            hint: "轻微偏暖",
            kelvin: Some(5500),
            color: None,
            swatch: "#fdf0e0",
        },
        Preset {
            id: "office",
            name: "办公",
            hint: "长时间写代码",
            kelvin: Some(4500),
            color: None,
            swatch: "#ffdfb8",
        },
        Preset {
            id: "night",
            name: "夜间",
            hint: "晚上看屏幕",
            kelvin: Some(3400),
            color: None,
            swatch: "#ffc48c",
        },
        Preset {
            id: "sleep",
            name: "睡前",
            hint: "睡前半小时",
            kelvin: Some(2700),
            color: None,
            swatch: "#ffab6b",
        },
        Preset {
            id: "green",
            name: "豆沙绿",
            hint: "老式护眼绿",
            kelvin: None,
            color: Some([199, 237, 204]),
            swatch: "#c7edcc",
        },
        Preset {
            id: "paper",
            name: "纸质暖黄",
            hint: "看文档不刺眼",
            kelvin: None,
            color: Some([250, 235, 210]),
            swatch: "#faebd2",
        },
    ]
}

/// 最终要下发给屏幕的东西：要么是色温，要么是一个具体的颜色。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tint {
    Kelvin { kelvin: u32 },
    Rgb { rgb: [u8; 3] },
}

/// 综合「开关 / 自动 / 预设 / 自定义」算出当前该用什么。
/// 返回 None 表示应该关掉效果、恢复原色。
pub fn resolve(cfg: &Config, minutes: f64) -> Option<Tint> {
    if !cfg.enabled {
        return None;
    }

    if cfg.schedule.enabled {
        return Some(Tint::Kelvin {
            kelvin: cfg.schedule.kelvin_at(minutes),
        });
    }

    if cfg.preset == PRESET_CUSTOM {
        return Some(match cfg.color {
            Some(rgb) => Tint::Rgb { rgb },
            None => Tint::Kelvin { kelvin: cfg.kelvin },
        });
    }

    if let Some(p) = presets().into_iter().find(|p| p.id == cfg.preset) {
        if let Some(rgb) = p.color {
            return Some(Tint::Rgb { rgb });
        }
        if let Some(kelvin) = p.kelvin {
            return Some(Tint::Kelvin { kelvin });
        }
    }

    Some(Tint::Kelvin { kelvin: cfg.kelvin })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 白天正午用日间色温() {
        let s = Schedule::default();
        assert_eq!(s.kelvin_at(12.0 * 60.0), 5500);
    }

    #[test]
    fn 深夜用夜间色温() {
        let s = Schedule::default();
        assert_eq!(s.kelvin_at(23.0 * 60.0), 3000);
        assert_eq!(s.kelvin_at(3.0 * 60.0), 3000);
    }

    #[test]
    fn 过渡区间是平滑的() {
        let s = Schedule::default();
        // 7:00 开始爬升，8:00 到白值，7:30 应该在中间
        let mid = s.kelvin_at(7.5 * 60.0);
        assert!(mid > 3000 && mid < 5500, "过渡中段应该落在两端之间，实际 {mid}");
        // 起点接近夜间值，终点接近日间值
        assert!(s.kelvin_at(7.0 * 60.0 + 1.0) < 3100);
        assert!(s.kelvin_at(7.99 * 60.0) > 5400);
    }

    #[test]
    fn 关闭时不下发任何调整() {
        let mut cfg = Config::default();
        cfg.enabled = false;
        assert!(resolve(&cfg, 12.0 * 60.0).is_none());
    }

    #[test]
    fn 自动模式优先于预设() {
        let mut cfg = Config::default();
        cfg.preset = "night".to_string();
        cfg.schedule.enabled = true;
        match resolve(&cfg, 12.0 * 60.0) {
            Some(Tint::Kelvin { kelvin }) => assert_eq!(kelvin, 5500),
            other => panic!("应该是白天色温，实际 {other:?}"),
        }
    }

    #[test]
    fn 自定义取色优先生效() {
        let mut cfg = Config::default();
        cfg.preset = PRESET_CUSTOM.to_string();
        cfg.color = Some([199, 237, 204]);
        match resolve(&cfg, 12.0 * 60.0) {
            Some(Tint::Rgb { rgb }) => assert_eq!(rgb, [199, 237, 204]),
            other => panic!("应该是自选颜色，实际 {other:?}"),
        }
    }
}
