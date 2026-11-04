//! 极简截屏工具，输出 BMP。
//!
//! 存在的唯一目的：在没有图形依赖的环境里，把屏幕拍下来用于验证界面。
//! 用法：screen-shot <输出路径>

use std::fs::File;
use std::io::{BufWriter, Write};
use std::mem::zeroed;
use std::ptr::{null_mut};

use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BI_RGB, DIB_RGB_COLORS, HDC, SRCCOPY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "shot.bmp".to_string());

    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            eprintln!("拿不到屏幕尺寸");
            return;
        }

        let screen: HDC = GetDC(null_mut());
        let mem: HDC = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let old = SelectObject(mem, bitmap);

        if BitBlt(mem, 0, 0, width, height, screen, 0, 0, SRCCOPY) == 0 {
            eprintln!("抓取失败");
            return;
        }

        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;

        let stride = width as usize * 4;
        let mut pixels = vec![0u8; stride * height as usize];

        let lines = GetDIBits(
            mem,
            bitmap,
            0,
            height as u32,
            pixels.as_mut_ptr() as *mut _,
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

        // GetDIBits 给的是自上而下的行序，BMP 文件要自下而上，这里翻一次
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
