//! 命令行验证工具。
//!
//! 这是外壳之前的「裸引擎」，用来在真机上验证行为，
//! 后面套 Tauri 界面时，调用的就是同一套 eyecare-core + eyecare-win。
//!
//! 用法：
//!   gamma-probe list                 看屏幕清单和 HDR 状态
//!   gamma-probe apply 4000,2700 4    临时生效 4 秒后恢复
//!   gamma-probe stay 3400            一直生效，每 5 秒复校一次，Ctrl+C 退出并恢复
//!   gamma-probe recover              上次被强杀留下的残留，手动收拾

use std::thread::sleep;
use std::time::Duration;

use eyecare_core::display::{
    clear_state, default_state_path, load_state, restore_all, save_state, snapshot,
};
use eyecare_core::{build_gamma_ramp, DisplayBackend};
use eyecare_win::{verify_roundtrip, WindowsBackend};

fn parse_kelvins(arg: Option<&String>) -> Vec<u32> {
    arg.map(|s| {
        s.split(',')
            .filter_map(|x| x.trim().parse::<u32>().ok())
            .collect()
    })
    .unwrap_or_default()
}

/// 如果上次没干净退出，先把屏幕救回来。
fn recover_if_needed(backend: &WindowsBackend) -> bool {
    let path = default_state_path();
    let saved = load_state(&path);
    if saved.is_empty() {
        return false;
    }
    println!(
        "[补救] 发现上次留下的备份，先把屏幕恢复原样：{}",
        path.display()
    );
    restore_all(backend, &saved);
    clear_state(&path);
    true
}

fn apply_all(backend: &WindowsBackend, kelvins: &[u32]) {
    let displays = backend.list();
    for (i, display) in displays.iter().enumerate() {
        let kelvin = kelvins[i.min(kelvins.len() - 1)];
        let ramp = build_gamma_ramp(kelvin, 1.0);
        let ok = backend.write_ramp(&display.id, &ramp);
        let verdict = match verify_roundtrip(backend, &display.id, &ramp) {
            Some(0) => "读回一致".to_string(),
            Some(d) => format!("读回偏差 {d}"),
            None => "读回失败".to_string(),
        };
        let hdr_warn = if display.hdr == Some(true) {
            "  ⚠ HDR 已开启，可能看不出变化"
        } else {
            ""
        };
        println!(
            "  {} {:>5}K  写入={}  {}{}",
            display.name, kelvin, ok, verdict, hdr_warn
        );
    }
}

fn main() {
    let backend = WindowsBackend::new();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("list");

    match cmd {
        "list" => {
            let displays = backend.list();
            println!("共 {} 块屏幕：", displays.len());
            for (i, d) in displays.iter().enumerate() {
                println!("  [{i}] {}  ({})", d.label(), d.id);
            }
            if displays.iter().any(|d| d.hdr == Some(true)) {
                println!("\n有屏幕开着 HDR，那块屏上 Gamma 表不起作用。");
            }
        }

        "recover" => {
            if !recover_if_needed(&backend) {
                println!("没有残留的备份文件：{}", default_state_path().display());
            }
        }

        // 看一眼每块屏当前的 Gamma 表被压到哪了。原始表三个峰值都是 65535。
        "check" => {
            for d in backend.list() {
                match backend.read_ramp(&d.id) {
                    Some(r) => {
                        println!(
                            "{}  峰值 R={} G={} B={}",
                            d.name,
                            r[255],
                            r[256 + 255],
                            r[512 + 255]
                        );
                    }
                    None => println!("{}  读不到", d.name),
                }
            }
        }

        "apply" | "stay" => {
            recover_if_needed(&backend);

            let kelvins = parse_kelvins(args.get(1));
            if kelvins.is_empty() {
                println!("色温参数不合法，例如：{cmd} 4000,2700 5");
                return;
            }
            if backend.list().is_empty() {
                println!("没有枚举到任何显示器");
                return;
            }

            // 落盘备份必须在改动屏幕之前完成
            let saved = snapshot(&backend);
            let path = default_state_path();
            if let Err(e) = save_state(&path, &saved) {
                println!("[警告] 备份写入失败：{e}（强杀后将无法自动恢复）");
            }

            if cmd == "apply" {
                let seconds: u64 = args
                    .get(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(3)
                    .clamp(1, 60);

                println!("\n下发（{seconds} 秒后恢复）：");
                apply_all(&backend, &kelvins);
                for left in (1..=seconds).rev() {
                    print!("\r  还有 {left} 秒恢复... ");
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    sleep(Duration::from_secs(1));
                }
                println!();
                restore_all(&backend, &saved);
                clear_state(&path);
                println!("[恢复] 已写回原始表");
            } else {
                // stay：常驻模式，同时挂一个 Ctrl+C 清理钩子
                let saved_for_exit = saved.clone();
                let path_for_exit = path.clone();
                let _ = ctrlc::set_handler(move || {
                    let be = WindowsBackend::new();
                    restore_all(&be, &saved_for_exit);
                    clear_state(&path_for_exit);
                    println!("\n[退出] 已恢复屏幕并清理备份");
                    std::process::exit(0);
                });

                println!("\n常驻生效中，按 Ctrl+C 退出：");
                loop {
                    apply_all(&backend, &kelvins);
                    sleep(Duration::from_secs(5));
                }
            }
        }

        // 检查 / 调整开机自启。自启登记的名称和主程序保持一致，都是 eyecare
        "autostart" => match args.get(1).map(|s| s.as_str()) {
            Some("on") => {
                let Some(exe) = args.get(2) else {
                    println!("用法：autostart on <exe 完整路径>");
                    return;
                };
                // 命令行里传进来的路径可能被 shell 转成正斜杠，
                // 注册表里必须是反斜杠，否则开机时根本启动不了
                let exe = exe.replace('/', "\\");
                let command = format!("\"{exe}\" --hidden");
                let ok = eyecare_win::autostart::set_enabled("eyecare", Some(&command));
                println!("写入结果 {ok}");
                println!("当前值 {:?}", eyecare_win::autostart::current_command("eyecare"));
            }
            Some("off") => {
                let ok = eyecare_win::autostart::set_enabled("eyecare", None);
                println!("删除结果 {ok}");
                println!("当前状态 {}", eyecare_win::autostart::is_enabled("eyecare"));
            }
            _ => {
                println!("状态 {}", eyecare_win::autostart::is_enabled("eyecare"));
                println!("命令 {:?}", eyecare_win::autostart::current_command("eyecare"));
            }
        },

        _ => {
            println!("用法：");
            println!("  gamma-probe list");
            println!("  gamma-probe apply <色温[,色温...]> [秒数]");
            println!("  gamma-probe stay  <色温[,色温...]>");
            println!("  gamma-probe check");
            println!("  gamma-probe recover");
            println!("  gamma-probe autostart [on <exe 路径> | off]");
        }
    }
}
