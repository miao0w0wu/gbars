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
use gba_core::gba::GameBoyAdvance;

fn main() -> ExitCode {
    // ---- 1. ROM 头部信息（第一个可选参数是 ROM 路径）----
    let args: Vec<String> = std::env::args().collect();
    let rom_path = args.get(1).filter(|p| !p.is_empty());
    match rom_path {
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

    // ---- 2. 有 ROM 时：跑 N 帧并输出 BMP（第二个参数是帧数，默认 300）----
    if let Some(path) = rom_path {
        // `--trace N`：单步执行前 N 条指令并打印（M2 调试用）。
        if let Some(pos) = args.iter().position(|a| a == "--trace") {
            let n: u64 = args.get(pos + 1).and_then(|s| s.parse().ok()).unwrap_or(64);
            match trace(path, n) {
                Ok(()) => return ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("运行失败: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        let frames: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
        let out = args.get(3).map(String::as_str).unwrap_or("frame.bmp");
        match run_frames(path, frames, out) {
            Ok(()) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("运行失败: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // ---- 3. 无 ROM：输出 SMPTE 测试帧，验证帧契约与落盘链路 ----
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

/// 单步执行前 `n` 条指令并打印 PC/模式/寄存器（M2 调试用）。
/// `--trace N [from]`：从第 `from` 步开始打印，便于定位故障点。
fn trace(rom: &str, n: u64) -> Result<(), Box<dyn std::error::Error>> {
    let mut gba = GameBoyAdvance::load_rom(rom)?;
    for i in 0..n {
        let t = gba.cpu.t();
        let pc = gba.cpu.reg(15) - if t { 4 } else { 8 };
        let insn = if t {
            gba.bus.read_u16(pc) as u32
        } else {
            gba.bus.read_u32(pc)
        };
        if i % 20000 == 0 {
            println!(
                "#{i:<8} PC={pc:#010x} {} {:?} insn={insn:#08x} r0={:#010x} r1={:#010x} r4={:#010x} cpsr={:#010x} vcount={}",
                if t { "TH" } else { "AR" },
                gba.cpu.mode(),
                gba.cpu.reg(0),
                gba.cpu.reg(1),
                gba.cpu.reg(4),
                gba.cpu.cpsr(),
                gba.ppu().vcount(),
            );
        }
        gba.step()?;
    }
    Ok(())
}

/// 跑 `frames` 帧，把最后一帧写成 BMP，并打印 PPU/CPU 状态摘要。
fn run_frames(rom: &str, frames: u32, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut gba = GameBoyAdvance::load_rom(rom)?;
    for i in 0..frames {
        if let Err(e) = gba.run_frame() {
            eprintln!("第 {i} 帧执行出错: {e}");
            eprintln!(
                "  当前 PC = {:#010x}，模式 = {:?}",
                gba.cpu.reg(15),
                gba.cpu.mode()
            );
            return Err(e.into());
        }
    }
    let frame = gba.frame().clone();
    write_bmp(out, &frame)?;
    let ppu = gba.ppu();
    println!(
        "跑完 {frames} 帧：DISPCNT={:#06x} VCOUNT={} 模式={} → {out}",
        ppu.dispcnt(),
        ppu.vcount(),
        ppu.dispcnt() & 0x7
    );
    println!(
        "  PC={:#010x} 模式={:?} steps={} TM0={:#06x} ctrl={:#04x}",
        gba.cpu.reg(15),
        gba.cpu.mode(),
        gba.cpu.steps(),
        gba.bus.read_u16(0x0400_0100),
        gba.bus.read_u16(0x0400_0102),
    );
    println!(
        "  handler={:#010x} IME={} IE={:#06x} IF={:#06x} DISPSTAT={:#06x}",
        gba.bus.read_u32(0x03FF_FFFC),
        gba.bus.read_u16(0x0400_0208),
        gba.bus.read_u16(0x0400_0200),
        gba.bus.read_u16(0x0400_0202),
        gba.bus.read_u16(0x0400_0004),
    );
    Ok(())
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
