//! 配置结构、预设清单、以及「现在该用什么颜色」的换算。
//!
//! 这一层不做任何系统调用，纯粹是数据，方便单测。
//!
//! 0.2 之前只有「白天一档、夜间一档」。现在改成一条由多个锚点组成的曲线：
//! 每个锚点是一个钟点加一个色温，锚点之间保持前一个色温，临近下一个锚点时才
//! 用 `transition` 分钟做线性过渡。这样既能表达多段变化，又不会让屏幕一整天
//! 都在缓慢变色。
//!
//! 老配置里没有 `points`，`normalize` 会用旧的 day/night 两档补出来，
//! 所以升级不会丢掉用户原来的设置。

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use eyecare_core::sun;

pub const PRESET_CUSTOM: &str = "custom";

pub const KELVIN_MIN: u32 = 1000;
pub const KELVIN_MAX: u32 = 6500;

/* ------------------------------------------------------------------ 当前时刻 */

/// 换算色温时需要的「现在的样子」。
///
/// 单独拆出来是因为它要同时回答三个问题：现在几点、今天是不是周末、
/// 今天是今年第几天（日出日落要用）。直接读系统时间会让单测没法固定时间。
#[derive(Debug, Clone, Copy)]
pub struct Moment {
    /// 一天中的分钟数，带小数
    pub minutes: f64,
    pub is_weekend: bool,
    pub day_of_year: u32,
    /// 当地时区相对 UTC 的分钟数，北京时间是 480
    pub utc_offset_min: i32,
}

impl Moment {
    pub fn now() -> Self {
        use chrono::{Datelike, Timelike};
        let now = chrono::Local::now();
        Self {
            minutes: now.hour() as f64 * 60.0 + now.minute() as f64 + now.second() as f64 / 60.0,
            // num_days_from_monday 里周六是 5、周日是 6
            is_weekend: now.weekday().num_days_from_monday() >= 5,
            day_of_year: now.ordinal(),
            utc_offset_min: local_offset_minutes(),
        }
    }

    /// 测试用：固定成一个工作日的某个时刻，时区按北京时间。
    #[cfg(test)]
    pub fn weekday_at(minutes: f64) -> Self {
        Self {
            minutes,
            is_weekend: false,
            day_of_year: 172,
            utc_offset_min: 480,
        }
    }

    #[cfg(test)]
    pub fn weekend_at(minutes: f64) -> Self {
        Self {
            is_weekend: true,
            ..Self::weekday_at(minutes)
        }
    }

    #[cfg(test)]
    pub fn on_day(mut self, day_of_year: u32) -> Self {
        self.day_of_year = day_of_year;
        self
    }
}

fn local_offset_minutes() -> i32 {
    // chrono 不直接给「本地时区相对 UTC 的分钟数」，用一次换算拿到
    use chrono::Local;
    let now = Local::now();
    let offset = now.offset().local_minus_utc();
    offset / 60
}

/* ------------------------------------------------------------------ 曲线锚点 */

/// 曲线上的一个点：从零点算起的分钟数 + 那一刻的目标色温。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub at: u32,
    pub kelvin: u32,
}

/// 按钟点算，还是跟着当地的日出日落走。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleMode {
    #[default]
    Clock,
    Sun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub enabled: bool,
    #[serde(default)]
    pub mode: ScheduleMode,
    /// 白天色温。`mode = sun` 时用这一档当白天的目标值。
    pub day_kelvin: u32,
    /// 夜间色温。`mode = sun` 时用这一档当夜间的目标值。
    pub night_kelvin: u32,
    /// 白天开始时刻，距零点的分钟数。只在迁移旧配置时用到。
    pub day_start: u32,
    /// 夜间开始时刻。只在迁移旧配置时用到。
    pub night_start: u32,
    /// 过渡时长，分钟。
    pub transition: u32,
    /// 工作日曲线，至少两个点。
    #[serde(default)]
    pub points: Vec<Anchor>,
    /// 周末曲线。空表示跟工作日一样。
    #[serde(default)]
    pub weekend_points: Vec<Anchor>,
    /// 观察点纬度，北纬为正。默认深圳。
    #[serde(default = "default_lat")]
    pub lat: f64,
    /// 观察点经度，东经为正。
    #[serde(default = "default_lon")]
    pub lon: f64,
    /// 日出日落前后各留多少分钟做过渡。
    #[serde(default = "default_sun_margin")]
    pub sun_margin: u32,
}

fn default_lat() -> f64 {
    22.55
}
fn default_lon() -> f64 {
    114.06
}
fn default_sun_margin() -> u32 {
    40
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ScheduleMode::Clock,
            day_kelvin: 5500,
            night_kelvin: 3000,
            day_start: 7 * 60,
            night_start: 19 * 60,
            transition: 60,
            // 新装用户直接给一条五段的曲线，比两档更能说明「这东西可以细调」
            points: vec![
                Anchor { at: 7 * 60, kelvin: 5500 },
                Anchor { at: 9 * 60, kelvin: 5200 },
                Anchor { at: 14 * 60, kelvin: 5000 },
                Anchor { at: 18 * 60 + 30, kelvin: 4000 },
                Anchor { at: 22 * 60 + 30, kelvin: 3200 },
            ],
            weekend_points: Vec::new(),
            lat: default_lat(),
            lon: default_lon(),
            sun_margin: default_sun_margin(),
        }
    }
}

impl Schedule {
    /// 把老配置补齐成新形状，并顺手把锚点排好序。
    pub fn normalize(&mut self) {
        if self.points.is_empty() {
            self.points = vec![
                Anchor {
                    at: self.day_start,
                    kelvin: self.day_kelvin,
                },
                Anchor {
                    at: self.night_start,
                    kelvin: self.night_kelvin,
                },
            ];
        }

        sort_points(&mut self.points);
        sort_points(&mut self.weekend_points);

        // 时间点重复会让插值算出 0 长度的区间，去重时留第一个
        self.points.dedup_by_key(|a| a.at);
        self.weekend_points.dedup_by_key(|a| a.at);

        self.lat = self.lat.clamp(-90.0, 90.0);
        self.lon = self.lon.clamp(-180.0, 180.0);
        self.sun_margin = self.sun_margin.min(180);
        self.transition = self.transition.min(180);
    }

    /// 这一刻该用多少色温。
    pub fn kelvin_at(&self, moment: &Moment) -> f64 {
        let points = self.points_for(moment);
        sample(&points, moment.minutes, self.transition)
    }

    fn points_for(&self, moment: &Moment) -> Vec<Anchor> {
        match self.mode {
            ScheduleMode::Sun => self.sun_points(moment),
            ScheduleMode::Clock => {
                if moment.is_weekend && !self.weekend_points.is_empty() {
                    self.weekend_points.clone()
                } else {
                    self.points.clone()
                }
            }
        }
    }

    /// 日出前后暖起来、日落前后暖下去，四个点交给统一的插值处理。
    fn sun_points(&self, moment: &Moment) -> Vec<Anchor> {
        let (sunrise, sunset) = match sun::sun_times(
            moment.day_of_year,
            self.lat,
            self.lon,
            moment.utc_offset_min,
        ) {
            Some(t) => (t.sunrise, t.sunset),
            // 极昼极夜没有日出日落，退回一个保守的固定时刻，
            // 总比让屏幕在一天里保持同一个颜色强
            None => (6.0 * 60.0, 18.0 * 60.0),
        };

        let margin = self.sun_margin as f64;
        let wrap = |m: f64| -> u32 {
            let v = ((m % 1440.0) + 1440.0) % 1440.0;
            (v.round() as u32) % 1440
        };

        vec![
            Anchor {
                at: wrap(sunrise - margin),
                kelvin: self.night_kelvin,
            },
            Anchor {
                at: wrap(sunrise + margin),
                kelvin: self.day_kelvin,
            },
            Anchor {
                at: wrap(sunset - margin),
                kelvin: self.day_kelvin,
            },
            Anchor {
                at: wrap(sunset + margin),
                kelvin: self.night_kelvin,
            },
        ]
    }

    /// 现在的曲线长什么样，给界面画图用。
    /// 日出模式返回的是今天算出来的四个点，不是配置里存的那条。
    pub fn effective_points(&self, moment: &Moment) -> Vec<Anchor> {
        let mut points = self.points_for(moment);
        sort_points(&mut points);
        points.dedup_by_key(|a| a.at);
        points
    }

    /// 今天的日出和日落（当地分钟），查不到时为空。
    pub fn sun_times(&self, moment: &Moment) -> Option<sun::SunTimes> {
        sun::sun_times(moment.day_of_year, self.lat, self.lon, moment.utc_offset_min)
    }
}

fn sort_points(points: &mut [Anchor]) {
    points.sort_by_key(|a| a.at);
}

fn lerp(from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * t.clamp(0.0, 1.0)
}

/// 在锚点序列上取值。
///
/// 规则是「保持 + 过渡」而不是「一路线性」：从当前锚点开始一直保持它的色温，
/// 直到距离下一个锚点还剩 `transition` 分钟时才开始变。
///
/// 锚点按一天 24 小时首尾相接，所以最后一个点到第一个点之间也是连续的一段。
fn sample(raw: &[Anchor], minutes: f64, transition: u32) -> f64 {
    let mut points = raw.to_vec();
    sort_points(&mut points);
    points.dedup_by_key(|a| a.at);

    if points.is_empty() {
        return KELVIN_MAX as f64;
    }
    if points.len() == 1 {
        return points[0].kelvin as f64;
    }

    let n = points.len();
    let m = ((minutes % 1440.0) + 1440.0) % 1440.0;

    // 找最后一个不晚于当前时刻的锚点；都比当前晚就落在最后一个点之后
    let mut idx = n - 1;
    for (i, point) in points.iter().enumerate() {
        if point.at as f64 <= m {
            idx = i;
        } else {
            break;
        }
    }

    let cur = points[idx];
    let next = points[(idx + 1) % n];

    // 跨零点时把「下一个」展开到第二天，这样区间长度始终是正的
    let cur_at = cur.at as f64;
    let next_at = if idx + 1 == n {
        next.at as f64 + 1440.0
    } else {
        next.at as f64
    };
    let into = if m >= cur_at {
        m - cur_at
    } else {
        m + 1440.0 - cur_at
    };

    let span = (next_at - cur_at).max(0.001);
    let tr = (transition as f64).min(span);

    if tr <= 0.0 || into < span - tr {
        return cur.kelvin as f64;
    }

    let t = (into - (span - tr)) / tr;
    lerp(cur.kelvin as f64, next.kelvin as f64, t)
}

/* ------------------------------------------------------------------ 护眼计时 */

/// 20-20-20：每工作 20 分钟，看 20 英尺外 20 秒。
///
/// 这里只管「什么时候该提醒」。到点后由主进程把屏幕短暂调暖，
/// 让提醒落到屏幕上，而不只是待在面板里。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakPlan {
    pub enabled: bool,
    pub work_minutes: u32,
    pub rest_seconds: u32,
    /// 到点时把屏幕短暂调暖压暗
    #[serde(default = "default_true")]
    pub dim_screen: bool,
    /// 下次提醒的时间戳。None 表示计时暂停或还没开始
    #[serde(default)]
    pub due_at: Option<u64>,
}

fn default_true() -> bool {
    true
}

impl Default for BreakPlan {
    fn default() -> Self {
        Self {
            enabled: true,
            work_minutes: 20,
            rest_seconds: 20,
            dim_screen: true,
            due_at: None,
        }
    }
}

impl BreakPlan {
    pub fn arm(&mut self) {
        if self.enabled && self.due_at.is_none() {
            self.due_at = Some(now_secs() + self.work_minutes.max(1) as u64 * 60);
        }
    }

    pub fn disarm(&mut self) {
        self.due_at = None;
    }

    pub fn remaining(&self) -> Option<u64> {
        self.due_at.map(|due| due.saturating_sub(now_secs()))
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/* -------------------------------------------------------------------- 配置 */

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
    /// 用户自己保存的预设
    #[serde(default)]
    pub extra_presets: Vec<Preset>,
    /// 被用户移出列表的内置预设 id
    #[serde(default)]
    pub hidden_presets: Vec<String>,
    pub schedule: Schedule,
    #[serde(default)]
    pub break_plan: BreakPlan,
}

fn default_theme() -> String {
    "system".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            // 首次装好时保持原色。一打开就把屏幕染暖，用户会以为是装坏了
            preset: "off".to_string(),
            kelvin: 6500,
            brightness: 1.0,
            color: None,
            target: "all".to_string(),
            theme: default_theme(),
            extra_presets: Vec::new(),
            hidden_presets: Vec::new(),
            schedule: Schedule::default(),
            break_plan: BreakPlan::default(),
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
        let mut cfg: Config = fs::read_to_string(config_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();

        // 旧配置只有 day/night 两档，这里补成锚点；之后所有逻辑只认锚点
        cfg.normalize();
        cfg.break_plan.arm();
        cfg
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

    pub fn normalize(&mut self) {
        self.schedule.normalize();
        self.kelvin = self.kelvin.clamp(KELVIN_MIN, KELVIN_MAX);
        self.brightness = self.brightness.clamp(0.05, 1.0);
    }
}

/* ---------------------------------------------------------------------- 预设 */

/// 一个预设：要么记色温，要么记一个具体颜色。
/// 字段改成 String 是为了支持用户自己新建和命名。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub hint: String,
    pub kelvin: Option<u32>,
    pub color: Option<[u8; 3]>,
    /// 列表里用来示意效果的颜色
    pub swatch: String,
}

fn p(
    id: &str,
    name: &str,
    hint: &str,
    kelvin: Option<u32>,
    color: Option<[u8; 3]>,
    swatch: &str,
) -> Preset {
    Preset {
        id: id.to_string(),
        name: name.to_string(),
        hint: hint.to_string(),
        kelvin,
        color,
        swatch: swatch.to_string(),
    }
}

/// 内置预设。用户可以把它移出列表，但定义始终保留，以后还能恢复。
pub fn builtin_presets() -> Vec<Preset> {
    vec![
        p("off", "原色", "不做任何调整", Some(6500), None, "#d7e2f4"),
        p("soft", "轻度", "轻微偏暖", Some(5500), None, "#f5e3c8"),
        p("office", "办公", "长时间写代码", Some(4500), None, "#ffd096"),
        p("night", "夜间", "晚上看屏幕", Some(3400), None, "#ffb173"),
        p("sleep", "睡前", "睡前半小时", Some(2700), None, "#ff8f4f"),
        p(
            "green",
            "豆沙绿",
            "老式护眼绿",
            None,
            Some([199, 237, 204]),
            "#a9ddb7",
        ),
        p(
            "paper",
            "纸质暖黄",
            "看文档不刺眼",
            None,
            Some([250, 235, 210]),
            "#f0d29f",
        ),
    ]
}

/// 界面上实际看到的列表：内置里没被隐藏的，加上用户自己保存的。
pub fn presets_for(cfg: &Config) -> Vec<Preset> {
    let mut all: Vec<Preset> = builtin_presets()
        .into_iter()
        .filter(|item| !cfg.hidden_presets.iter().any(|id| id == &item.id))
        .collect();
    all.extend(cfg.extra_presets.iter().cloned());
    all
}

pub fn find_preset(cfg: &Config, id: &str) -> Option<Preset> {
    presets_for(cfg).into_iter().find(|item| item.id == id)
}

/// 色温换算成示意色，新建预设时用它生成小圆点。
pub fn swatch_for_kelvin(kelvin: u32) -> String {
    const STOPS: [(u32, [u8; 3]); 5] = [
        (2700, [0xff, 0x8f, 0x4f]),
        (3400, [0xff, 0xb1, 0x73]),
        (4500, [0xff, 0xd0, 0x96]),
        (5500, [0xf5, 0xe3, 0xc8]),
        (6500, [0xd7, 0xe2, 0xf4]),
    ];

    let k = kelvin.clamp(2700, 6500);
    for pair in STOPS.windows(2) {
        let (lo_k, lo_c) = pair[0];
        let (hi_k, hi_c) = pair[1];
        if k <= hi_k {
            let t = (k - lo_k) as f64 / (hi_k - lo_k) as f64;
            let mix = |i: usize| {
                (lo_c[i] as f64 + (hi_c[i] as f64 - lo_c[i] as f64) * t).round() as u8
            };
            return format!("#{:02x}{:02x}{:02x}", mix(0), mix(1), mix(2));
        }
    }
    "#d7e2f4".to_string()
}

pub fn swatch_for_rgb(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
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
pub fn resolve(cfg: &Config, moment: &Moment) -> Option<Tint> {
    if !cfg.enabled {
        return None;
    }

    // 自动模式优先：开了「按时间/日出日落」就不看预设
    if cfg.schedule.enabled {
        let kelvin = cfg
            .schedule
            .kelvin_at(moment)
            .round()
            .clamp(KELVIN_MIN as f64, KELVIN_MAX as f64) as u32;
        return Some(Tint::Kelvin { kelvin });
    }

    if cfg.preset == PRESET_CUSTOM {
        return Some(match cfg.color {
            Some(rgb) => Tint::Rgb { rgb },
            None => Tint::Kelvin { kelvin: cfg.kelvin },
        });
    }

    if let Some(item) = find_preset(cfg, &cfg.preset) {
        if let Some(rgb) = item.color {
            return Some(Tint::Rgb { rgb });
        }
        if let Some(kelvin) = item.kelvin {
            return Some(Tint::Kelvin { kelvin });
        }
    }

    Some(Tint::Kelvin { kelvin: cfg.kelvin })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 老配置只有两档时补成两个锚点() {
        let mut schedule = Schedule::default();
        schedule.points.clear();
        schedule.day_start = 7 * 60;
        schedule.night_start = 19 * 60;
        schedule.day_kelvin = 5500;
        schedule.night_kelvin = 3000;

        schedule.normalize();

        assert_eq!(schedule.points.len(), 2);
        assert_eq!(schedule.points[0], Anchor { at: 420, kelvin: 5500 });
        assert_eq!(schedule.points[1], Anchor { at: 1140, kelvin: 3000 });
    }

    #[test]
    fn 白天和深夜取到平台值() {
        let s = Schedule::default();
        assert_eq!(s.kelvin_at(&Moment::weekday_at(12.0 * 60.0)), 5200.0);
        assert_eq!(s.kelvin_at(&Moment::weekday_at(23.5 * 60.0)), 3200.0);
    }

    #[test]
    fn 过渡只在临近下一个锚点前发生() {
        let s = Schedule::default();
        assert_eq!(s.kelvin_at(&Moment::weekday_at(12.0 * 60.0)), 5200.0);
        assert_eq!(s.kelvin_at(&Moment::weekday_at(13.0 * 60.0)), 5200.0);
        let mid = s.kelvin_at(&Moment::weekday_at(13.5 * 60.0));
        assert!(mid > 5000.0 && mid < 5200.0, "过渡中途应在两档之间，实际 {mid}");
    }

    #[test]
    fn 跨零点不会断档() {
        let s = Schedule::default();
        assert_eq!(s.kelvin_at(&Moment::weekday_at(23.0 * 60.0)), 3200.0);
        assert_eq!(s.kelvin_at(&Moment::weekday_at(1.0 * 60.0)), 3200.0);

        let dawn = s.kelvin_at(&Moment::weekday_at(6.5 * 60.0));
        assert!(dawn > 3200.0 && dawn < 5500.0, "实际 {dawn}");
    }

    #[test]
    fn 周末曲线只在周末生效() {
        let mut s = Schedule::default();
        s.weekend_points = vec![
            Anchor { at: 9 * 60, kelvin: 6000 },
            Anchor { at: 23 * 60, kelvin: 3800 },
        ];

        assert_eq!(s.kelvin_at(&Moment::weekday_at(12.0 * 60.0)), 5200.0);
        assert_eq!(s.kelvin_at(&Moment::weekend_at(12.0 * 60.0)), 6000.0);
    }

    #[test]
    fn 日出模式白天用白天的色温() {
        let mut s = Schedule::default();
        s.mode = ScheduleMode::Sun;
        s.day_kelvin = 5600;
        s.night_kelvin = 2900;

        let noon = Moment::weekday_at(12.0 * 60.0).on_day(172);
        assert_eq!(s.kelvin_at(&noon), 5600.0);
    }

    #[test]
    fn 日出模式给出四个锚点() {
        let mut s = Schedule::default();
        s.mode = ScheduleMode::Sun;
        let points = s.effective_points(&Moment::weekday_at(12.0 * 60.0).on_day(172));
        assert_eq!(points.len(), 4);
        assert_eq!(points[0].kelvin, s.night_kelvin);
        assert!(points[1].at < points[2].at, "日出应早于日落");
    }

    #[test]
    fn 关闭时不下发任何调整() {
        let mut cfg = Config::default();
        cfg.enabled = false;
        assert!(resolve(&cfg, &Moment::weekday_at(12.0 * 60.0)).is_none());
    }

    #[test]
    fn 自动模式优先于预设() {
        let mut cfg = Config::default();
        cfg.preset = "night".to_string();
        cfg.schedule.enabled = true;
        match resolve(&cfg, &Moment::weekday_at(12.0 * 60.0)) {
            Some(Tint::Kelvin { kelvin }) => assert_eq!(kelvin, 5200),
            other => panic!("应该是白天色温，实际 {other:?}"),
        }
    }

    #[test]
    fn 自定义取色优先生效() {
        let mut cfg = Config::default();
        cfg.preset = PRESET_CUSTOM.to_string();
        cfg.color = Some([199, 237, 204]);
        match resolve(&cfg, &Moment::weekday_at(12.0 * 60.0)) {
            Some(Tint::Rgb { rgb }) => assert_eq!(rgb, [199, 237, 204]),
            other => panic!("应该是自选颜色，实际 {other:?}"),
        }
    }

    #[test]
    fn 用户预设追加在列表末尾并能生效() {
        let mut cfg = Config::default();
        cfg.extra_presets
            .push(p("u1", "我的", "自己保存的", Some(5000), None, "#ffd0a0"));

        let list = presets_for(&cfg);
        assert_eq!(list.last().unwrap().id, "u1");

        cfg.preset = "u1".to_string();
        match resolve(&cfg, &Moment::weekday_at(12.0 * 60.0)) {
            Some(Tint::Kelvin { kelvin }) => assert_eq!(kelvin, 5000),
            other => panic!("用户预设应该生效，实际 {other:?}"),
        }
    }

    #[test]
    fn 移出的内置预设不再出现在列表里() {
        let mut cfg = Config::default();
        cfg.hidden_presets.push("night".to_string());

        let list = presets_for(&cfg);
        assert!(list.iter().all(|item| item.id != "night"));
        assert!(list.iter().any(|item| item.id == "office"));
    }

    #[test]
    fn 色温示意色在两端对齐预设配色() {
        assert_eq!(swatch_for_kelvin(6500), "#d7e2f4");
        assert_eq!(swatch_for_kelvin(2700), "#ff8f4f");
        assert_eq!(swatch_for_kelvin(500), "#ff8f4f");
    }

    #[test]
    fn 计时器按工作分钟数设定到期时间() {
        let mut plan = BreakPlan::default();
        plan.arm();
        let remaining = plan.remaining().unwrap();
        assert!(remaining > 19 * 60 && remaining <= 20 * 60, "实际 {remaining}");
        plan.disarm();
        assert!(plan.remaining().is_none());
    }
}
