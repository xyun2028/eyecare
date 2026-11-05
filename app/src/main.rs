#![windows_subsystem = "windows"]

mod config;
mod stats;

use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use config::{
    builtin_presets, find_preset, now_secs, presets_for, resolve, swatch_for_kelvin,
    swatch_for_rgb, Anchor, Config, Moment, Preset, Tint,
};
use eyecare_core::display::{
    clear_state, default_state_path, load_state, restore_all, save_state, snapshot,
};
use eyecare_core::{build_gamma_ramp, build_gamma_ramp_rgb, DisplayBackend, DisplayInfo};
use eyecare_win::WindowsBackend;
use stats::Stats;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};

/// 休息提示时把色温压到当前的这个比例，亮度再乘下面那个系数。
/// 用相对值而不是固定色温，是因为用户本来就在 2700K 时，
/// 一个固定的 1900K 几乎看不出变化，压三成才有「屏幕变了」的感觉。
const REST_KELVIN_RATIO: f64 = 0.55;
const REST_BRIGHTNESS_RATIO: f64 = 0.7;
/// 滤色模式下没有色温可压，退回这个暖色。
const REST_FALLBACK_KELVIN: u32 = 1800;

struct AppState {
    backend: WindowsBackend,
    config: Mutex<Config>,
    saved: Mutex<Vec<(String, [u16; 768])>>,
    /// 当前是否已经改过屏幕。用来避免「关闭状态」下反复覆盖用户的手动调整
    active: Mutex<bool>,
    /// 休息提示截止的时间戳。有值且还没到期时，写屏走「休息色」
    rest_until: Mutex<Option<u64>>,
    stats: Mutex<Stats>,
}

const APP_NAME: &str = "eyecare";
const WINDOW_TITLE: &str = "Eye眼";

#[derive(Clone, serde::Serialize)]
struct SunView {
    sunrise: f64,
    sunset: f64,
}

#[derive(Clone, serde::Serialize)]
struct ScheduleView {
    /// 今天实际生效的曲线（日出模式下是算出来的四个点）
    effective_points: Vec<Anchor>,
    sun_times: Option<SunView>,
    weekend: bool,
}

#[derive(Clone, serde::Serialize)]
struct BreakView {
    remaining_seconds: Option<u64>,
    resting: bool,
}

#[derive(Clone, serde::Serialize)]
struct Status {
    config: Config,
    presets: Vec<Preset>,
    displays: Vec<DisplayInfo>,
    effective: Option<Tint>,
    clock: String,
    schedule_view: ScheduleView,
    break_view: BreakView,
    stats: stats::StatsView,
}

fn clock_text() -> String {
    use chrono::Timelike;
    let now = chrono::Local::now();
    format!("{:02}:{:02}", now.hour(), now.minute())
}

fn status_of(state: &AppState, cfg: Config) -> Status {
    let moment = Moment::now();
    let effective = resolve(&cfg, &moment);
    let visible = presets_for(&cfg);

    let sun_times = cfg.schedule.sun_times(&moment).map(|t| SunView {
        sunrise: t.sunrise,
        sunset: t.sunset,
    });

    Status {
        config: cfg,
        presets: visible,
        displays: state.backend.list(),
        effective,
        clock: clock_text(),
        schedule_view: ScheduleView {
            effective_points: state_schedule_points(state, &moment),
            sun_times,
            weekend: moment.is_weekend,
        },
        break_view: BreakView {
            remaining_seconds: state.config.lock().unwrap().break_plan.remaining(),
            resting: rest_active(state),
        },
        stats: state.stats.lock().unwrap().view(),
    }
}

fn state_schedule_points(state: &AppState, moment: &Moment) -> Vec<Anchor> {
    state
        .config
        .lock()
        .unwrap()
        .schedule
        .effective_points(moment)
}

/// 休息提示是否还在生效。过期的顺手清掉。
fn rest_active(state: &AppState) -> bool {
    let mut guard = state.rest_until.lock().unwrap();
    match *guard {
        Some(until) if now_secs() < until => true,
        Some(_) => {
            *guard = None;
            false
        }
        None => false,
    }
}

fn begin_rest(state: &AppState, seconds: u64) {
    *state.rest_until.lock().unwrap() = Some(now_secs() + seconds.max(1));
}

/// 休息提示要把屏幕变成什么样。
///
/// 在原来的颜色上再往暖处压一档，而不是换成一个固定值：
/// 固定值在用户本来就很暖的时候看不出来，在用户很白的时候又太突兀。
fn rest_tint(tint: Tint, brightness: f64) -> (Tint, f64) {
    let kelvin = match tint {
        Tint::Kelvin { kelvin } => (kelvin as f64 * REST_KELVIN_RATIO).round() as u32,
        Tint::Rgb { .. } => REST_FALLBACK_KELVIN,
    };
    (
        Tint::Kelvin {
            kelvin: kelvin.clamp(config::KELVIN_MIN, config::KELVIN_MAX),
        },
        (brightness * REST_BRIGHTNESS_RATIO).clamp(0.05, 1.0),
    )
}

/// 把当前配置真正写到屏幕上。
fn apply_now(state: &AppState, cfg: &Config) {
    if !cfg.enabled {
        let mut active = state.active.lock().unwrap();
        if *active {
            restore_all(&state.backend, &state.saved.lock().unwrap());
            *active = false;
        }
        return;
    }

    let Some(mut tint) = resolve(cfg, &Moment::now()) else {
        return;
    };

    let mut brightness = cfg.brightness.clamp(0.05, 1.0);
    let resting = rest_active(state);
    if resting {
        let (next_tint, next_brightness) = rest_tint(tint, brightness);
        tint = next_tint;
        brightness = next_brightness;
    }

    *state.active.lock().unwrap() = true;
    let saved = state.saved.lock().unwrap();

    for display in state.backend.list() {
        let targeted = cfg.target == "all" || cfg.target == display.id;
        if targeted {
            let ramp = match tint {
                Tint::Kelvin { kelvin } => build_gamma_ramp(kelvin, brightness),
                Tint::Rgb { rgb } => build_gamma_ramp_rgb(rgb, brightness),
            };
            state.backend.write_ramp(&display.id, &ramp);
        } else if let Some((_, original)) = saved.iter().find(|(id, _)| id == &display.id) {
            state.backend.write_ramp(&display.id, original);
        }
    }
}

/// 拉起看门狗。它只等主进程退出，发现备份还在就把屏幕写回原样。
/// 没有它的话，任务管理器一结束进程，屏幕就永远停在偏色状态。
fn spawn_watchdog(state_path: &std::path::Path) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(dir) = exe.parent() else {
        return;
    };
    let dog = dir.join("eyecare-watchdog.exe");
    if !dog.exists() {
        return;
    }
    let _ = std::process::Command::new(dog)
        .arg("--parent")
        .arg(std::process::id().to_string())
        .arg("--state")
        .arg(state_path)
        .spawn();
}

/// 退出前把屏幕还原，并清掉崩溃恢复用的备份。
/// 这里不能用 unwrap：锁一旦中毒就 panic，反而让恢复代码根本跑不到。
fn shutdown(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    if let Ok(saved) = state.saved.lock() {
        restore_all(&state.backend, &saved);
    }
    if let Ok(stats) = state.stats.lock() {
        stats.save();
    }
    clear_state(&default_state_path());
}

#[tauri::command]
fn bootstrap(state: tauri::State<AppState>) -> Status {
    let cfg = state.config.lock().unwrap().clone();
    status_of(&state, cfg)
}

/// 把主题同步给窗口本身，标题栏颜色才会跟着变。
/// "system" 对应 None，交给系统决定。
fn apply_window_theme(app: &tauri::AppHandle, mode: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let theme = match mode {
            "light" => Some(tauri::Theme::Light),
            "dark" => Some(tauri::Theme::Dark),
            _ => None,
        };
        let _ = window.set_theme(theme);
    }
}

#[tauri::command]
fn update_config(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    cfg: Config,
) -> Status {
    let old = state.config.lock().unwrap().clone();
    let mut safe = cfg.clone();
    safe.normalize();

    // 间隔改了要重新计时，否则用户把 20 分钟改成 5 分钟，还得等满旧的 20 分钟
    if safe.break_plan.work_minutes != old.break_plan.work_minutes {
        safe.break_plan.disarm();
    }
    if !safe.break_plan.enabled {
        safe.break_plan.disarm();
    } else {
        safe.break_plan.arm();
    }

    safe.save();
    *state.config.lock().unwrap() = safe.clone();
    apply_now(&state, &safe);
    apply_window_theme(&app, &safe.theme);
    status_of(&state, safe)
}

/// 手动结束正在进行的休息提示（屏幕还停在暖色时用）。
#[tauri::command]
fn end_rest(state: tauri::State<AppState>) -> Status {
    *state.rest_until.lock().unwrap() = None;
    let cfg = state.config.lock().unwrap().clone();
    apply_now(&state, &cfg);
    status_of(&state, cfg)
}

/// 把之前移出列表的内置预设全部找回来。用户自己存的预设不受影响。
#[tauri::command]
fn restore_presets(app: tauri::AppHandle, state: tauri::State<AppState>) -> Status {
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.hidden_presets.clear();
    cfg.save();
    *state.config.lock().unwrap() = cfg.clone();
    refresh_tray_menu(&app, &cfg);
    status_of(&state, cfg)
}

#[tauri::command]
fn get_autostart() -> bool {
    eyecare_win::autostart::is_enabled(APP_NAME)
}

/// 把「当前生效的颜色」存成一个具名预设。
/// 自动模式开着的时候，存的是此刻自动算出来的那个色温。
#[tauri::command]
fn save_preset(app: tauri::AppHandle, state: tauri::State<AppState>, name: String) -> Status {
    let mut cfg = state.config.lock().unwrap().clone();

    let (kelvin, color, swatch) = match resolve(&cfg, &Moment::now()) {
        Some(Tint::Kelvin { kelvin }) => (Some(kelvin), None, swatch_for_kelvin(kelvin)),
        Some(Tint::Rgb { rgb }) => (None, Some(rgb), swatch_for_rgb(rgb)),
        None => (Some(cfg.kelvin), None, swatch_for_kelvin(cfg.kelvin)),
    };

    let trimmed: String = name.trim().chars().take(12).collect();
    let label = if trimmed.is_empty() {
        "未命名".to_string()
    } else {
        trimmed
    };

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let id = format!("u{stamp}");

    cfg.extra_presets.push(Preset {
        id: id.clone(),
        name: label,
        hint: "自己保存的".to_string(),
        kelvin,
        color,
        swatch,
    });
    cfg.preset = id;
    cfg.enabled = true;

    cfg.save();
    *state.config.lock().unwrap() = cfg.clone();
    apply_now(&state, &cfg);
    refresh_tray_menu(&app, &cfg);
    status_of(&state, cfg)
}

/// 把预设移出列表。内置的只记进隐藏名单，定义还在，以后能恢复。
/// 如果移掉的正是当前在用的，效果不撤销，只是转成「自定义」——
/// 否则用户点一下删除，屏幕会突然跳回原色。
#[tauri::command]
fn delete_preset(app: tauri::AppHandle, state: tauri::State<AppState>, id: String) -> Status {
    let mut cfg = state.config.lock().unwrap().clone();
    let removed = find_preset(&cfg, &id);
    let was_active = cfg.preset == id;

    if builtin_presets().iter().any(|item| item.id == id) {
        if !cfg.hidden_presets.iter().any(|x| x == &id) {
            cfg.hidden_presets.push(id.clone());
        }
    } else {
        cfg.extra_presets.retain(|item| item.id != id);
    }

    if was_active {
        cfg.preset = config::PRESET_CUSTOM.to_string();
        if let Some(item) = removed {
            match (item.kelvin, item.color) {
                (_, Some(rgb)) => cfg.color = Some(rgb),
                (Some(kelvin), None) => {
                    cfg.kelvin = kelvin;
                    cfg.color = None;
                }
                _ => {}
            }
        }
    }

    cfg.save();
    *state.config.lock().unwrap() = cfg.clone();
    apply_now(&state, &cfg);
    refresh_tray_menu(&app, &cfg);
    status_of(&state, cfg)
}

/// 开：把「当前 exe 路径 + --hidden」写进注册表。
/// 关：删掉那个值。返回值代表注册表里真实的结果，不是我们想设的值。
#[tauri::command]
fn set_autostart(on: bool) -> bool {
    if on {
        match std::env::current_exe() {
            Ok(path) => {
                let command = format!("\"{}\" --hidden", path.display());
                eyecare_win::autostart::set_enabled(APP_NAME, Some(&command))
            }
            Err(_) => false,
        }
    } else {
        eyecare_win::autostart::set_enabled(APP_NAME, None)
    }
}

/// 托盘菜单要跟着预设列表走，所以抽成函数，增删预设后重建一次。
fn build_tray_menu(app: &tauri::AppHandle, cfg: &Config) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", "打开设置", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "暂停 / 恢复", true, None::<&str>)?;

    let preset_items: Vec<MenuItem<tauri::Wry>> = presets_for(cfg)
        .iter()
        .map(|item| {
            MenuItem::with_id(
                app,
                format!("preset:{}", item.id),
                item.name.as_str(),
                true,
                None::<&str>,
            )
            .unwrap()
        })
        .collect();
    let preset_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = preset_items
        .iter()
        .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
        .collect();
    let preset_menu = Submenu::with_items(app, "预设", true, &preset_refs)?;

    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    Menu::with_items(
        app,
        &[
            &show,
            &toggle,
            &preset_menu,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )
}

fn refresh_tray_menu(app: &tauri::AppHandle, cfg: &Config) {
    if let Some(tray) = app.tray_by_id("main") {
        if let Ok(menu) = build_tray_menu(app, cfg) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn main() {
    // 开机自启时带 --hidden，启动后直接缩在托盘里，不弹窗
    let start_hidden = std::env::args().any(|arg| arg == "--hidden");

    // 已经有实例在跑就把它的窗口叫出来，然后自己退出
    if !eyecare_win::acquire_single_instance("eyecare-app-instance") {
        eyecare_win::focus_window(WINDOW_TITLE);
        return;
    }

    tauri::Builder::default()
        .setup(move |app| {
            let backend = WindowsBackend::new();

            // 上次如果是被强杀的，磁盘上会留着备份，先把屏幕救回来
            let state_path = default_state_path();
            let leftover = load_state(&state_path);
            if !leftover.is_empty() {
                restore_all(&backend, &leftover);
                clear_state(&state_path);
            }

            let saved = snapshot(&backend);
            if save_state(&state_path, &saved).is_ok() {
                // 备份落盘成功，后面才敢动屏幕；同时拉起看门狗守着
                spawn_watchdog(&state_path);
            }

            let cfg = Config::load();
            app.manage(AppState {
                backend,
                config: Mutex::new(cfg.clone()),
                saved: Mutex::new(saved),
                active: Mutex::new(false),
                rest_until: Mutex::new(None),
                stats: Mutex::new(Stats::load()),
            });

            apply_now(&app.state::<AppState>(), &cfg);
            apply_window_theme(app.handle(), &cfg.theme);

            // 窗口在配置里是隐藏创建的，这里按启动方式决定要不要露出来。
            // 必须在这一步就 show：Windows 不会给「从未显示过」的窗口分配任务栏按钮。
            if let Some(window) = app.get_webview_window("main") {
                if start_hidden {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }

            // ---- 托盘 ----
            let menu = build_tray_menu(app.handle(), &cfg)?;

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Eye眼")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref().to_string();
                    match id.as_str() {
                        "quit" => {
                            shutdown(app);
                            app.exit(0);
                        }
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "toggle" => {
                            let state = app.state::<AppState>();
                            let mut cfg = state.config.lock().unwrap().clone();
                            cfg.enabled = !cfg.enabled;
                            cfg.save();
                            *state.config.lock().unwrap() = cfg.clone();
                            apply_now(&state, &cfg);
                            let _ = app.emit("config-changed", status_of(&state, cfg));
                        }
                        other if other.starts_with("preset:") => {
                            let preset_id = other.trim_start_matches("preset:").to_string();
                            let state = app.state::<AppState>();
                            let mut cfg = state.config.lock().unwrap().clone();
                            cfg.preset = preset_id;
                            cfg.enabled = true;
                            cfg.save();
                            *state.config.lock().unwrap() = cfg.clone();
                            apply_now(&state, &cfg);
                            let _ = app.emit("config-changed", status_of(&state, cfg));
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            // ---- 后台循环 ----
            // 每 2 秒重新下发一次：既是为了跟随曲线变化，
            // 也是为了防止系统或驱动把 Gamma 表重置掉。
            // 顺带记用眼时长、检查休息计时。
            let handle = app.handle().clone();
            thread::spawn(move || {
                let mut tick: u32 = 0;
                loop {
                    thread::sleep(Duration::from_secs(2));
                    let state = handle.state::<AppState>();
                    let now = now_secs();
                    let mut cfg = state.config.lock().unwrap().clone();

                    // 关掉护眼时不计时，但心跳照走，否则重新打开会把中间那段也算进来
                    state.stats.lock().unwrap().tick(now, cfg.enabled);

                    if cfg.break_plan.enabled {
                        if let Some(due) = cfg.break_plan.due_at {
                            if now >= due {
                                let rest = cfg.break_plan.rest_seconds.max(1) as u64;

                                // 面板可能根本没人看着，所以提醒要落到屏幕上
                                if cfg.break_plan.dim_screen && cfg.enabled {
                                    begin_rest(&state, rest);
                                }
                                state.stats.lock().unwrap().took_break();

                                cfg.break_plan.due_at =
                                    Some(now + cfg.break_plan.work_minutes.max(1) as u64 * 60);
                                cfg.save();
                                *state.config.lock().unwrap() = cfg.clone();

                                let _ = handle.emit(
                                    "break-due",
                                    serde_json::json!({ "restSeconds": rest }),
                                );
                            }
                        }
                    }

                    apply_now(&state, &cfg);

                    // 每 30 秒把最新状态推给界面，界面不用自己算时间
                    tick += 1;
                    if tick % 15 == 0 {
                        state.stats.lock().unwrap().save();
                        let _ = handle.emit("status", status_of(&state, cfg));
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // 关窗口不退出程序，缩回托盘
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            update_config,
            save_preset,
            delete_preset,
            restore_presets,
            end_rest,
            get_autostart,
            set_autostart
        ])
        .build(tauri::generate_context!())
        .expect("启动失败")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                shutdown(app);
            }
        });
}
