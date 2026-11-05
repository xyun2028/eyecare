//! 极简截屏工具，输出 BMP。
//!
//! 存在的唯一目的：在没有图形依赖的环境里，把屏幕拍下来用于验证界面。
//!
//! 用法：
//!   screen-shot <输出路径>                 全屏
//!   screen-shot <输出路径> <x> <y> <宽> <高>  只截一块区域

use std::fs::File;
use std::io::{BufWriter, Write};
use std::mem::zeroed;
use std::ptr::null_mut;

use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BI_RGB, DIB_RGB_COLORS, HDC, SRCCOPY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetSystemMetrics, GetWindowRect, SM_CXSCREEN, SM_CYSCREEN,
};

/// 只读窗口尺寸，不截图。用来核对窗口有没有被系统强制拉大，避免无谓地拍用户屏幕。
fn print_rect(title: &str) -> bool {
    let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide.as_ptr());
        if hwnd.is_null() {
            println!("没找到标题为「{title}」的窗口");
            return true;
        }
        let mut rect: windows_sys::Win32::Foundation::RECT = zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            println!("读不到窗口位置");
            return true;
        }
        println!(
            "窗口 {}x{}，位于 ({}, {})",
            rect.right - rect.left,
            rect.bottom - rect.top,
            rect.left,
            rect.top
        );
    }
    true
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.first().map(|s| s.as_str()) == Some("--rect") {
        let title = args.get(1).cloned().unwrap_or_else(|| "护眼 · eyecare".to_string());
        print_rect(&title);
        return;
    }

    let out = args.first().cloned().unwrap_or_else(|| "shot.bmp".to_string());

    // 可选裁切区域：x y 宽 高
    let crop: Option<(i32, i32, i32, i32)> = if args.len() >= 5 {
        let nums: Vec<i32> = args[1..5]
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() == 4 {
            Some((nums[0], nums[1], nums[2], nums[3]))
        } else {
            None
        }
    } else {
        None
    };

    unsafe {
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        if screen_w <= 0 || screen_h <= 0 {
            eprintln!("拿不到屏幕尺寸");
            return;
        }

        let screen: HDC = GetDC(null_mut());
        let mem: HDC = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, screen_w, screen_h);
        let old = SelectObject(mem, bitmap);

        if BitBlt(mem, 0, 0, screen_w, screen_h, screen, 0, 0, SRCCOPY) == 0 {
            eprintln!("抓取失败");
            return;
        }

        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize =
            std::mem::size_of::<windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = screen_w;
        info.bmiHeader.biHeight = -screen_h;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;

        let full_stride = screen_w as usize * 4;
        let mut full = vec![0u8; full_stride * screen_h as usize];

        let lines = GetDIBits(
            mem,
            bitmap,
            0,
            screen_h as u32,
            full.as_mut_ptr() as *mut _,
            &mut info,
            DIB_RGB_COLORS,
        );

        SelectObject(mem, old);
        DeleteObject(bitmap);
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);

        if lines == 0 {
            eprintln!("读取像素失败");
            return;
        }

        let (mut width, mut height) = (screen_w, screen_h);
        let mut pixels = full;

        if let Some((x, y, w, h)) = crop {
            let x = x.max(0).min(screen_w);
            let y = y.max(0).min(screen_h);
            let w = w.min(screen_w - x);
            let h = h.min(screen_h - y);
            if w <= 0 || h <= 0 {
                eprintln!("裁切区域无效");
                return;
            }

            let mut cropped = vec![0u8; w as usize * 4 * h as usize];
            for row in 0..h as usize {
                let src = (y as usize + row) * full_stride + x as usize * 4;
                let dst = row * w as usize * 4;
                cropped[dst..dst + w as usize * 4]
                    .copy_from_slice(&pixels[src..src + w as usize * 4]);
            }
            pixels = cropped;
            width = w;
            height = h;
        }

        // GetDIBits 给的是自上而下的行序，BMP 文件要自下而上，这里翻一次
        let stride = width as usize * 4;
        let mut rows: Vec<&[u8]> = pixels.chunks(stride).collect();
        rows.reverse();
        let mut flipped = Vec::with_capacity(pixels.len());
        for row in rows {
            flipped.extend_from_slice(row);
        }
        let pixels = flipped;

        let file_size = 54 + pixels.len() as u32;
        let mut file = BufWriter::new(File::create(&out).expect("无法创建文件"));

        // BITMAPFILEHEADER
        file.write_all(b"BM").unwrap();
        file.write_all(&file_size.to_le_bytes()).unwrap();
        file.write_all(&0u16.to_le_bytes()).unwrap();
        file.write_all(&0u16.to_le_bytes()).unwrap();
        file.write_all(&54u32.to_le_bytes()).unwrap();

        // BITMAPINFOHEADER
        file.write_all(&40u32.to_le_bytes()).unwrap();
        file.write_all(&width.to_le_bytes()).unwrap();
        file.write_all(&height.to_le_bytes()).unwrap();
        file.write_all(&1u16.to_le_bytes()).unwrap();
        file.write_all(&32u16.to_le_bytes()).unwrap();
        file.write_all(&0u32.to_le_bytes()).unwrap(); // BI_RGB
        file.write_all(&(pixels.len() as u32).to_le_bytes()).unwrap();
        file.write_all(&2835u32.to_le_bytes()).unwrap();
        file.write_all(&2835u32.to_le_bytes()).unwrap();
        file.write_all(&0u32.to_le_bytes()).unwrap();
        file.write_all(&0u32.to_le_bytes()).unwrap();

        file.write_all(&pixels).unwrap();
        file.flush().unwrap();

        println!("已保存 {out}  {width}x{height}");
    }
}
