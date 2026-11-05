#![windows_subsystem = "windows"]

mod config;

use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use config::{presets, resolve, Config, Preset, Tint};
use eyecare_core::display::{
    clear_state, default_state_path, load_state, restore_all, save_state, snapshot,
};
use eyecare_core::{build_gamma_ramp, build_gamma_ramp_rgb, DisplayBackend, DisplayInfo};
use eyecare_win::WindowsBackend;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};

struct AppState {
    backend: WindowsBackend,
    config: Mutex<Config>,
    saved: Mutex<Vec<(String, [u16; 768])>>,
    /// 当前是否已经改过屏幕。用来避免「关闭状态」下反复覆盖用户的手动调整
    active: Mutex<bool>,
}

const APP_NAME: &str = "eyecare";
const WINDOW_TITLE: &str = "Eye眼";

#[derive(Clone, serde::Serialize)]
struct Status {
    config: Config,
    presets: Vec<Preset>,
    displays: Vec<DisplayInfo>,
    effective: Option<Tint>,
    clock: String,
}

fn clock_text() -> String {
    use chrono::Timelike;
    let now = chrono::Local::now();
    format!("{:02}:{:02}", now.hour(), now.minute())
}

fn status_of(state: &AppState, cfg: Config) -> Status {
    let effective = resolve(&cfg, config::minutes_now());
    Status {
        config: cfg,
        presets: presets(),
        displays: state.backend.list(),
        effective,
        clock: clock_text(),
    }
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

    let tint = resolve(cfg, config::minutes_now());
    let Some(tint) = tint else {
        return;
    };

    *state.active.lock().unwrap() = true;
    let saved = state.saved.lock().unwrap();
    let brightness = cfg.brightness.clamp(0.05, 1.0);

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
    let mut safe = cfg.clone();
    safe.brightness = safe.brightness.clamp(0.05, 1.0);
    safe.kelvin = safe.kelvin.clamp(1000, 6500);

    safe.save();
    *state.config.lock().unwrap() = safe.clone();
    apply_now(&state, &safe);
    apply_window_theme(&app, &safe.theme);
    status_of(&state, safe)
}

#[tauri::command]
fn get_autostart() -> bool {
    eyecare_win::autostart::is_enabled(APP_NAME)
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
            let show = MenuItem::with_id(app, "show", "打开设置", true, None::<&str>)?;
            let toggle = MenuItem::with_id(app, "toggle", "暂停 / 恢复", true, None::<&str>)?;

            let preset_items: Vec<MenuItem<tauri::Wry>> = presets()
                .iter()
                .map(|p| {
                    MenuItem::with_id(
                        app,
                        format!("preset:{}", p.id),
                        p.name,
                        true,
                        None::<&str>,
                    )
                    .unwrap()
                })
                .collect();
            let preset_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = preset_items
                .iter()
                .map(|i| i as &dyn IsMenuItem<tauri::Wry>)
                .collect();
            let preset_menu = Submenu::with_items(app, "预设", true, &preset_refs)?;

            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &show,
                    &toggle,
                    &preset_menu,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?;

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
            // 每 2 秒重新下发一次：既是为了跟随时间自动变化，
            // 也是为了防止系统或驱动把 Gamma 表重置掉。
            let handle = app.handle().clone();
            thread::spawn(move || {
                let mut tick: u32 = 0;
                loop {
                    thread::sleep(Duration::from_secs(2));
                    let state = handle.state::<AppState>();
                    let cfg = state.config.lock().unwrap().clone();
                    apply_now(&state, &cfg);

                    // 每 30 秒把最新状态推给界面，界面不用自己算时间
                    tick += 1;
                    if tick % 15 == 0 && cfg.schedule.enabled {
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
