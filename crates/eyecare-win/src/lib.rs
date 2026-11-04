//! Windows 平台的显示器后端。
//!
//! 三件事：
//! 1. 逐屏下发 Gamma 表（用显示器设备名单独取 DC）
//! 2. 查出每块屏是否开了 HDR（开了的话 Gamma 表写了也没用）
//! 3. 开机自启、单实例守卫这些周边能力

pub mod autostart;

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use eyecare_core::{DisplayBackend, DisplayInfo};
use windows_sys::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, LPARAM, RECT, TRUE,
};
use windows_sys::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows_sys::Win32::UI::ColorSystem::{GetDeviceGammaRamp, SetDeviceGammaRamp};

/// MONITORINFOF_PRIMARY 在 windows-sys 里没有导出，值固定是 1。
const MONITORINFOF_PRIMARY: u32 = 1;

pub struct WindowsBackend;

impl WindowsBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsBackend {
    fn default() -> Self {
        Self::new()
    }
}

fn utf16_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 给一块屏单独取 DC。
/// 这是「逐屏不同色温」的关键：拿桌面 DC 会一次改掉所有屏幕。
fn open_dc(device: &str) -> Option<HDC> {
    let wide = to_wide(device);
    let hdc = unsafe { CreateDCW(wide.as_ptr(), null(), null(), null()) };
    if hdc.is_null() {
        None
    } else {
        Some(hdc)
    }
}

struct MonitorCtx {
    monitors: Vec<(String, u32, u32, bool)>,
}

unsafe extern "system" fn monitor_cb(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> i32 {
    let ctx = &mut *(lparam as *mut MonitorCtx);
    let mut info: MONITORINFOEXW = zeroed();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    if GetMonitorInfoW(hmon, &mut info as *mut _ as *mut _) != 0 {
        let id = utf16_to_string(&info.szDevice);
        if !id.is_empty() {
            let rc = info.monitorInfo.rcMonitor;
            let primary = info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;
            ctx.monitors.push((
                id,
                (rc.right - rc.left) as u32,
                (rc.bottom - rc.top) as u32,
                primary,
            ));
        }
    }
    TRUE
}

fn enumerate_monitors() -> Vec<(String, u32, u32, bool)> {
    let mut ctx = MonitorCtx {
        monitors: Vec::new(),
    };
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(monitor_cb),
            &mut ctx as *mut _ as LPARAM,
        );
    }
    ctx.monitors
}

/// 查每块屏的 HDR 状态，键是 GDI 设备名（`\\.\DISPLAY1`）。
///
/// 走 DisplayConfig 接口：屏幕作为「target」有高级色彩信息，
/// 同一路径上的「source」能给出对应的 GDI 名字，两边配对就能对上号。
fn hdr_map() -> HashMap<String, bool> {
    let mut map = HashMap::new();
    unsafe {
        let mut path_count: u32 = 0;
        let mut mode_count: u32 = 0;
        if GetDisplayConfigBufferSizes(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            &mut mode_count,
        ) != 0
        {
            return map;
        }

        let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![zeroed(); path_count as usize];
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![zeroed(); mode_count as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            null_mut(),
        ) != 0
        {
            return map;
        }

        for path in paths.iter().take(path_count as usize) {
            let mut source: DISPLAYCONFIG_SOURCE_DEVICE_NAME = zeroed();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let gdi_name = utf16_to_string(&source.viewGdiDeviceName);
            if gdi_name.is_empty() {
                continue;
            }

            let mut color: DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO = zeroed();
            color.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
            color.header.size = size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32;
            color.header.adapterId = path.targetInfo.adapterId;
            color.header.id = path.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut color.header) != 0 {
                continue;
            }

            // 第 0 位：支持高级色彩。第 1 位：当前已启用。
            // 只有「支持」才记录，不支持的话这个值没有意义。
            let bits = color.Anonymous.value;
            let supported = bits & 1 != 0;
            let enabled = bits & 2 != 0;
            if supported {
                map.insert(gdi_name, enabled);
            }
        }
    }
    map
}

impl DisplayBackend for WindowsBackend {
    fn list(&self) -> Vec<DisplayInfo> {
        let hdr = hdr_map();
        enumerate_monitors()
            .into_iter()
            .map(|(id, width, height, primary)| {
                let short = id.trim_start_matches(r"\\.\").to_string();
                DisplayInfo {
                    hdr: hdr.get(&id).copied(),
                    name: if short.is_empty() { id.clone() } else { short },
                    id,
                    width,
                    height,
                    primary,
                }
            })
            .collect()
    }

    fn read_ramp(&self, id: &str) -> Option<[u16; 768]> {
        let hdc = open_dc(id)?;
        let mut buf = [0u16; 768];
        let ok = unsafe { GetDeviceGammaRamp(hdc, buf.as_mut_ptr() as *mut c_void) } != 0;
        unsafe { DeleteDC(hdc) };
        if ok {
            Some(buf)
        } else {
            None
        }
    }

    fn write_ramp(&self, id: &str, ramp: &[u16; 768]) -> bool {
        let Some(hdc) = open_dc(id) else {
            return false;
        };
        let ok = unsafe { SetDeviceGammaRamp(hdc, ramp.as_ptr() as *const c_void) } != 0;
        unsafe { DeleteDC(hdc) };
        ok
    }
}

/// 写入后读回，返回最大偏差。
/// 0 表示系统原样收下了，但**不代表肉眼能看到变化**——HDR 下就是这种情况。
pub fn verify_roundtrip(backend: &WindowsBackend, id: &str, expect: &[u16; 768]) -> Option<i32> {
    let back = backend.read_ramp(id)?;
    Some(
        (0..768)
            .map(|i| (back[i] as i32 - expect[i] as i32).abs())
            .max()
            .unwrap_or(0),
    )
}

/// 同一时刻只允许一个实例在跑。
///
/// 用命名互斥体：第二个实例创建同名互斥体会失败，靠这个判断。
/// 两个实例同时改 Gamma 表会互相覆盖，还会出现「退一个把另一个的效果也还原了」。
///
/// 返回值 true 表示「你是第一个，继续跑」。互斥体句柄故意不释放，
/// 让它跟着进程一起消失——这正好就是我们需要的行为。
pub fn acquire_single_instance(name: &str) -> bool {
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let wide = to_wide(name);
    unsafe {
        let handle = CreateMutexW(null(), 1, wide.as_ptr());
        if handle.is_null() {
            // 连互斥体都建不出来，别把用户挡在门外
            return true;
        }
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// 把已经存在的窗口叫到前面来。第二个实例启动时用它把老窗口顶出来。
pub fn focus_window(title: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_SHOW,
    };

    let wide = to_wide(title);
    unsafe {
        let hwnd = FindWindowW(null(), wide.as_ptr());
        if hwnd.is_null() {
            return false;
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        true
    }
}
