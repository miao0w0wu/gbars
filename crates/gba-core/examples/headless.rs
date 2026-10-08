//! headless CLI：不开任何窗口运行模拟核心。
//!
//! 这是整个项目最重要的调试与回归测试入口——CI 里没有屏幕，但核心必须
//! 能被验证。它的能力会随里程碑成长：
//!   M0 装载 ROM、打印头部、输出测试帧位图（当前）；
//!   M1 单步执行、查看寄存器；
//!   M2+ 跑 N 帧、输出画面与帧哈希做图形回归。
//!
//! # 用法
//!
//! ```text
//! cargo run -p gba-core --example headless -- "D:\Games\gba roms\Pokemon.gba"
//! cargo run -p gba-core --example headless      # 不带参数：只输出测试帧
//! ```

use std::fs;
use std::process::ExitCode;

use gba_core::cartridge::Cartridge;
use gba_core::frame::{FRAME_BYTES, Frame, SCREEN_HEIGHT, SCREEN_WIDTH};

fn main() -> ExitCode {
    // ---- 1. ROM 头部信息（第一个可选参数是 ROM 路径）----
    let args: Vec<String> = std::env::args().collect();
    match args.get(1) {
        Some(path) => match Cartridge::load(path) {
            Ok(cart) => {
                let h = cart.header();
                println!("ROM 文件:   {path}");
                println!("标题:       {}", h.title);
                println!("游戏代码:   {}", h.game_code);
                println!("厂商代码:   {}", h.maker_code);
                println!("版本:       v1.{}", h.version);
                println!(
                    "大小:       {} 字节 ({:.2} MiB)",
                    cart.rom().len(),
                    cart.rom().len() as f64 / 1024.0 / 1024.0
                );
                println!(
                    "头部校验和: {}",
                    if h.checksum_valid {
                        "通过"
                    } else {
                        "不通过（ROM 可能被修改过或损坏）"
                    }
                );
            }
            Err(e) => {
                eprintln!("装载 ROM 失败: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => println!("（未指定 ROM 路径，跳过头部解析）"),
    }

    // ---- 2. 测试帧 → BMP：验证帧契约与落盘链路 ----
    let frame = Frame::test_pattern();
    match write_bmp("test-frame.bmp", &frame) {
        Ok(()) => println!(
            "测试帧已写入 test-frame.bmp（{SCREEN_WIDTH}×{SCREEN_HEIGHT}，{FRAME_BYTES} 字节/帧）"
        ),
        Err(e) => {
            eprintln!("写出测试帧失败: {e}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// 把 RGBA 帧写成 24 位 BMP 文件。
///
/// 不引入图像库（核心保持零依赖，见 docs/design.md §9），BMP 足够简单，
/// 适合手写：14 字节文件头 + 40 字节信息头 + 像素数据。
///
/// BMP 有两个反直觉的"怪癖"（历史上与 OS/2 显示卡的位序有关），代码中
/// 对应位置都有注释：
/// 1. 像素行**从下到上**存储（先写屏幕最后一行）；
/// 2. 颜色字节序是 **B, G, R**（不是 R, G, B）。
fn write_bmp(path: &str, frame: &Frame) -> std::io::Result<()> {
    let width = SCREEN_WIDTH as u32;
    let height = SCREEN_HEIGHT as u32;
    // BMP 要求每行字节数向上对齐到 4 的倍数
    // （240 像素 × 3 字节 = 720，恰好整除，但按通用规则计算）
    let row_padding = (4 - (width as usize * 3) % 4) % 4;
    let row_bytes = width as usize * 3 + row_padding;
    let pixel_bytes = row_bytes * height as usize;
    let file_size = 14 + 40 + pixel_bytes;

    let mut out = Vec::with_capacity(file_size);

    // ---- BITMAPFILEHEADER（14 字节）----
    out.extend_from_slice(b"BM"); // 文件魔数
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // 保留字段，必须为 0
    out.extend_from_slice(&(14u32 + 40).to_le_bytes()); // 像素数据偏移（文件头+信息头）

    // ---- BITMAPINFOHEADER（40 字节）----
    out.extend_from_slice(&40u32.to_le_bytes()); // 本头的大小
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // 颜色平面数，恒为 1
    out.extend_from_slice(&24u16.to_le_bytes()); // 位深：24bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // 压缩方式：BI_RGB（不压缩）
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes()); // 水平分辨率（72 DPI）
    out.extend_from_slice(&2835u32.to_le_bytes()); // 垂直分辨率
    out.extend_from_slice(&0u32.to_le_bytes()); // 调色板颜色数（真彩无调色板）
    out.extend_from_slice(&0u32.to_le_bytes()); // "重要"颜色数

    // ---- 像素数据：bottom-up + BGR ----
    let px = frame.pixels();
    for y in (0..height as usize).rev() {
        // 从最底一行写到最顶一行
        for x in 0..width as usize {
            let i = (y * SCREEN_WIDTH + x) * 4;
            out.push(px[i + 2]); // B
            out.push(px[i + 1]); // G
            out.push(px[i]); // R
        }
        // 行尾对齐填充：把 Vec 扩到指定长度，新增部分填 0
        out.resize(out.len() + row_padding, 0);
    }

    // 自检：装配出的字节数应与文件头声明一致（仅调试构建生效）
    debug_assert_eq!(out.len(), file_size);
    fs::write(path, out)
}
