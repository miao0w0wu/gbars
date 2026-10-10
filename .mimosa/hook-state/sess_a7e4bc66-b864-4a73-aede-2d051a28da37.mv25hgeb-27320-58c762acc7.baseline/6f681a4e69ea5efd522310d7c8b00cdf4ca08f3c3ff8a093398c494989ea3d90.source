//! BIOS 高层模拟（HLE）：把游戏的 BIOS 调用直接模拟掉，而不是跳进
//! 一个不存在的 BIOS。
//!
//! # 硬件背景：真机上 SWI 指令发生什么
//!
//! SWI（软件中断）触发异常：CPU 切到 Supervisor 模式、跳到 BIOS 的
//! 0x08 向量，BIOS 按立即数（SWI 编码里的 24 位注释字段）分发到
//! 对应的服务例程——除法、内存拷贝、图形解压……——执行完通过
//! `SUB pc, lr, #0` 回到调用者。BIOS 是 16 KiB 的 Nintendo 固件，
//! 我们没有它，也不打算提供（法律原因）。
//!
//! # HLE 的思路：让 SWI 变成"一条普通指令"
//!
//! 既然 BIOS 例程只是"计算结果 + 返回"，我们可以在 SWI 执行的瞬间
//! **直接算出结果**：改写 r0–r3 / 内存，然后让 PC 像普通指令一样
//! 推进——调用者完全感知不到中间发生过什么（模式、标志都不动，
//! 这也符合真机语义：游戏在 SVC 态跑 BIOS 是瞬间的）。
//!
//! 代价与已知限制（都标注在对应函数）：
//! - **IntrWait/VBlankIntrWait 暂时立即返回**——真 BIOS 会挂起等待
//!   中断，唤醒机制依赖 M3 的中断控制器；
//! - Halt/Stop 只置标志，唤醒同样在 M3；
//! - 未知/未实现的 SWI 按 no-op 处理（绝不让 PC 跳进零 BIOS）。
//!
//! # 实现来源
//!
//! 每个函数对照 mGBA `src/gba/bios.c` 的同名 HLE 与 GBATEK
//! "GBA BIOS Routines" 章。返回值约定（GBATEK）：结果放 r0，
//! 辅助值 r1/r3（调用方常直接复用，不要多写寄存器）。

use crate::bus::Bus;
use crate::cpu::Cpu;

/// SWI 主分发。`comment` 是指令编码里的立即数（ARM 24 位 / Thumb 8 位）。
pub(crate) fn hle_swi(cpu: &mut Cpu, bus: &mut Bus, comment: u32) {
    match comment {
        0x00 => soft_reset(cpu, bus),
        0x01 => register_ram_reset(cpu, bus),
        0x02 => cpu.halted = true, // Halt：M3 中断就绪后可被唤醒
        0x03 => {}                 // Stop：桩（真机进入深度睡眠，游戏极少用）
        // IntrWait / VBlankIntrWait：游戏主循环的心跳。
        // 语义（GBATEK）：清除 IF 中指定的位 → 挂起 → 对应中断到达时唤醒。
        // VBlankIntrWait 还要求"清掉已 pending 的旧 VBlank 再等新的"。
        0x04 => intr_wait(cpu, bus, cpu.reg(1) as u16, false),
        0x05 => intr_wait(cpu, bus, 1, true),
        0x06 => div(cpu, cpu.reg(0) as i32, cpu.reg(1) as i32),
        0x07 => div(cpu, cpu.reg(1) as i32, cpu.reg(0) as i32),
        0x08 => cpu.regs[0] = isqrt(cpu.reg(0)),
        0x09 => arctan(cpu),
        0x0A => arctan2(cpu),
        0x0B => cpu_set(cpu, bus, false),
        0x0C => cpu_set(cpu, bus, true),
        0x0D => {
            cpu.regs[0] = 0xBAAE_187F; // 真机 BIOS 校验和（GBATEK 常数）
            cpu.regs[1] = 1;
            cpu.regs[3] = 0x4000;
        }
        0x0E => bg_affine_set(cpu, bus),
        0x0F => obj_affine_set(cpu, bus),
        0x10 => bit_unpack(cpu, bus),
        0x11 => lz77(cpu, bus, 1), // UnCompWram：按字节写
        0x12 => lz77(cpu, bus, 2), // UnCompVram：按半字合成写
        0x13 => rl(cpu, bus, 1),
        0x14 => rl(cpu, bus, 2),
        0x15 => huffman(cpu, bus),
        // Sound driver 系列（0x16–0x1E）：m4a 引擎的初始化/播放控制。
        // 引擎本体在游戏 ROM 里，BIOS 只是薄入口——no-op 即可用。
        // TODO(M4)：MidiKey2Freq 需要真实现，其余保持 no-op。
        0x16..=0x1E => {}
        0x1F => midi_key2freq(cpu, bus),
        _ => {} // 未知 SWI：no-op，绝不跳进零 BIOS
    }
}

/// SoftReset：真机重启回 BIOS 入口。无 BIOS 模式下等效于"整机复位
/// 回 ROM 启动"——把 CPU 恢复成无 BIOS 启动状态。
fn soft_reset(cpu: &mut Cpu, _bus: &mut Bus) {
    *cpu = Cpu::new_no_bios();
    cpu.pc_written = true;
}

/// IntrWait 真语义：清 IF 中关心的位 → 挂起（Halt）→ 中断唤醒。
/// VBlankIntrWait 的要点：即使 VBlank 已 pending 也要先清掉再等新的
/// （官方 BIOS 行为，游戏靠它对齐帧）。
fn intr_wait(cpu: &mut Cpu, bus: &mut Bus, clear_mask: u16, vblank: bool) {
    let mask = if vblank { 1 | clear_mask } else { clear_mask };
    bus.io.clear_irq(mask);
    cpu.halted = true;
}

/// RegisterRamReset：按 r0 位掩码清空各内存区（游戏启动时的例行打扫）。
fn register_ram_reset(cpu: &mut Cpu, bus: &mut Bus) {
    let mask = cpu.reg(0);
    // 真机 BIOS 先把屏幕关成强制空白（mGBA 同款行为）
    bus.write_u16(0x0400_0000, 0x0080);
    if mask & 0x01 != 0 {
        clear_mem(bus, 0x0200_0000, 0x0040_0000);
    }
    if mask & 0x02 != 0 {
        // IWRAM 顶部 0x200 字节是 IRQ 栈，BIOS 不清（GBATEK）
        clear_mem(bus, 0x0300_0000, 0x0000_7E00);
    }
    if mask & 0x04 != 0 {
        clear_mem(bus, 0x0500_0000, 0x0000_0400);
    }
    if mask & 0x08 != 0 {
        clear_mem(bus, 0x0600_0000, 0x0001_8000);
    }
    if mask & 0x0_10 != 0 {
        clear_mem(bus, 0x0700_0000, 0x0000_0400);
    }
    // 位 5–7（串行/声音/其它寄存器）：TODO(M4) IO 寄存器语义就绪后补全
}

/// 向连续区域逐字节写 0。区域大小都是常量，编译器能优化成 memset。
fn clear_mem(bus: &mut Bus, base: u32, size: u32) {
    for i in 0..size {
        bus.write_u8(base + i, 0);
    }
}

/// Div / DivArm：有符号除法。结果：r0 = 商、r1 = 余数、r3 = |商|。
///
/// 除零在真机上不异常（ARM 没有除法异常），结果是垃圾——mGBA 的
/// HLE 用"尽力而为"的值代替，避免游戏崩溃。
fn div(cpu: &mut Cpu, num: i32, denom: i32) {
    if denom == 0 {
        // 除零：abs(num) <= 1 时真机行为近似可预期，其余本应挂死
        cpu.regs[0] = if num < 0 { -1i32 as u32 } else { 1 };
        cpu.regs[1] = num as u32;
        cpu.regs[3] = 1;
    } else if denom == -1 && num == i32::MIN {
        // INT_MIN / -1 在 Rust 里 panic，真机返回 INT_MIN（x86 风格）
        cpu.regs[0] = i32::MIN as u32;
        cpu.regs[1] = 0;
        cpu.regs[3] = i32::MIN as u32;
    } else {
        let quot = num / denom;
        let rem = num % denom;
        cpu.regs[0] = quot as u32;
        cpu.regs[1] = rem as u32;
        cpu.regs[3] = quot.unsigned_abs();
    }
}

/// 整数平方根（floor）。经典"二进制逐位试商"算法，无浮点误差。
fn isqrt(x: u32) -> u32 {
    let mut rem = x;
    let mut root = 0u32;
    let mut bit = 1u32 << 30; // 从最高能落的位置开始（4 位的倍数）
    while bit > rem {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root
}

/// ArcTan：输入 fix 2.14 定点数，输出 fix 2.14 弧度近似（多项式拟合）。
/// 常数来自 GBATEK；系数链与 mGBA 完全一致。
fn arctan(cpu: &mut Cpu) {
    let i = cpu.reg(0) as i32;
    let a = -((i.wrapping_mul(i)) >> 14);
    let mut b = ((0xA9i32.wrapping_mul(a)) >> 14).wrapping_add(0x390);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0x91C);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0xFB6);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0x16AA);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0x2081);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0x3651);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0xA2F9);
    cpu.regs[0] = (i.wrapping_mul(b) >> 16) as i16 as u32;
    cpu.regs[1] = a as u32;
    cpu.regs[3] = b as u32;
}

/// ArcTan2：四象限反正切，输出 0..0x20000 的定点角（2^16 = 一整圈）。
fn arctan2(cpu: &mut Cpu) {
    let x = cpu.reg(0) as i32;
    let y = cpu.reg(1) as i32;
    let result: i32 = if y == 0 {
        if x >= 0 { 0 } else { 0x8000 }
    } else if x == 0 {
        if y >= 0 { 0x4000 } else { 0xC000 }
    } else if y >= 0 {
        if x >= 0 {
            if x >= y {
                arctan_poly((y << 14) / x)
            } else {
                0x4000 - arctan_poly((x << 14) / y)
            }
        } else if -x <= y {
            0x4000 + arctan_poly((-y << 14) / x)
        } else {
            0x8000 - arctan_poly((y << 14) / -x)
        }
    } else if x >= 0 {
        if x >= -y {
            0x1_0000 - arctan_poly((-y << 14) / x)
        } else {
            0xC000 + arctan_poly((x << 14) / -y)
        }
    } else if -x <= -y {
        0x8000 + arctan_poly((y << 14) / x)
    } else {
        0xC000 - arctan_poly((-x << 14) / -y)
    };
    cpu.regs[0] = (result as u16) as u32;
    cpu.regs[1] = result as u32;
    cpu.regs[3] = 0x170;
}

/// ArcTan 的多项式主体（供 arctan / arctan2 复用）。
fn arctan_poly(i: i32) -> i32 {
    let a = -((i.wrapping_mul(i)) >> 14);
    let mut b = ((0xA9i32.wrapping_mul(a)) >> 14).wrapping_add(0x390);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0x91C);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0xFB6);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0x16AA);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0x2081);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0x3651);
    b = ((b.wrapping_mul(a)) >> 14).wrapping_add(0xA2F9);
    (i.wrapping_mul(b) >> 16) as i16 as i32
}

/// CpuSet / CpuFastSet：批量拷贝或填充。
///
/// r0=源（fill 模式下是要填的值）、r1=目标、r2=控制字：
/// `count = r2 & 0x1FFFFF`，`bit24 = fill`，CpuSet 的 `bit26 = 32 位`
/// （否则 16 位）；CpuFastSet 恒 32 位、按 8 字（32 字节）块搬运。
fn cpu_set(cpu: &mut Cpu, bus: &mut Bus, fast: bool) {
    let src = cpu.reg(0);
    let dst = cpu.reg(1);
    let ctrl = cpu.reg(2);
    let fill = ctrl & (1 << 24) != 0;
    let count = ctrl & 0x001F_FFFF;

    if fast {
        // CpuFastSet：count 以字节计，向上取整到 8 字块（真机行为）
        let words = count / 4;
        let blocks = words.div_ceil(8);
        if fill {
            for i in 0..blocks * 8 {
                bus.write_u32(dst + i * 4, src);
            }
        } else {
            for i in 0..blocks * 8 {
                let value = bus.read_u32(src + i * 4);
                bus.write_u32(dst + i * 4, value);
            }
        }
    } else {
        let wide = ctrl & (1 << 26) != 0;
        if wide {
            if fill {
                for i in 0..count {
                    bus.write_u32(dst + i * 4, src);
                }
            } else {
                for i in 0..count {
                    let value = bus.read_u32(src + i * 4);
                    bus.write_u32(dst + i * 4, value);
                }
            }
        } else if fill {
            let half = src as u16;
            for i in 0..count {
                bus.write_u16(dst + i * 2, half);
            }
        } else {
            for i in 0..count {
                let value = bus.read_u16(src + i * 2);
                bus.write_u16(dst + i * 2, value);
            }
        }
    }
}

/// BgAffineSet：批量计算背景仿射参数（旋转 + 缩放 + 平移）。
/// 输入结构 20 字节/组，输出 16 字节/组（OAM 风格定点 8.8）。
fn bg_affine_set(cpu: &mut Cpu, bus: &mut Bus) {
    let mut offset = cpu.reg(0);
    let mut dest = cpu.reg(1);
    let mut i = cpu.reg(2) as i32;
    while i > 0 {
        i -= 1;
        // 原点（fix 20.12）与屏幕中心、缩放（8.8）、角度（1 字节 = 一圈）
        let ox = bus.read_u32(offset) as i32 as f32 / 256.0;
        let oy = bus.read_u32(offset + 4) as i32 as f32 / 256.0;
        let cx = bus.read_u16(offset + 8) as i16 as f32;
        let cy = bus.read_u16(offset + 10) as i16 as f32;
        let sx = bus.read_u16(offset + 12) as i16 as f32 / 256.0;
        let sy = bus.read_u16(offset + 14) as i16 as f32 / 256.0;
        let theta = (bus.read_u16(offset + 16) >> 8) as f32 / 128.0 * std::f32::consts::PI;
        offset += 20;

        // 旋转矩阵 × 缩放（与 mGBA 的浮点公式一致）
        let (sin, cos) = theta.sin_cos();
        let a = cos * sx;
        let b = -sin * sx;
        let c = sin * sy;
        let d = cos * sy;
        let rx = ox - (a * cx + b * cy);
        let ry = oy - (c * cx + d * cy);

        bus.write_u16(dest, (a * 256.0) as i16 as u16);
        bus.write_u16(dest + 2, (b * 256.0) as i16 as u16);
        bus.write_u16(dest + 4, (c * 256.0) as i16 as u16);
        bus.write_u16(dest + 6, (d * 256.0) as i16 as u16);
        bus.write_u32(dest + 8, (rx * 256.0) as i32 as u32);
        bus.write_u32(dest + 12, (ry * 256.0) as i32 as u32);
        dest += 16;
    }
}

/// ObjAffineSet：精灵仿射参数。与 BgAffineSet 的差别：没有平移项，
/// 且四个矩阵元素按 `diff` 间距写到目标（通常写入 OAM 的仿射组）。
fn obj_affine_set(cpu: &mut Cpu, bus: &mut Bus) {
    let mut offset = cpu.reg(0);
    let mut dest = cpu.reg(1);
    let diff = cpu.reg(3);
    let mut i = cpu.reg(2) as i32;
    while i > 0 {
        i -= 1;
        let sx = bus.read_u16(offset) as i16 as f32 / 256.0;
        let sy = bus.read_u16(offset + 2) as i16 as f32 / 256.0;
        let theta = (bus.read_u16(offset + 4) >> 8) as f32 / 128.0 * std::f32::consts::PI;
        offset += 8;

        let (sin, cos) = theta.sin_cos();
        let a = cos * sx;
        let b = -sin * sx;
        let c = sin * sy;
        let d = cos * sy;

        bus.write_u16(dest, (a * 256.0) as i16 as u16);
        bus.write_u16(dest + diff, (b * 256.0) as i16 as u16);
        bus.write_u16(dest + diff * 2, (c * 256.0) as i16 as u16);
        bus.write_u16(dest + diff * 3, (d * 256.0) as i16 as u16);
        dest += diff * 4;
    }
}

/// BitUnPack：把窄位宽的数据流展开成宽位宽（如 4 位色板 → 8 位索引）。
/// 控制块：[0..2] 源字节数、[2] 源位宽、[3] 目标位宽、[4..8] 偏移值
/// （其位 31 = "仅当源值非零才加偏移"）。
/// 源位流 LSB 优先；因为源位宽只能是 1/2/4/8，每个单元恰好落在
/// 单个字节内（这正是真机限制位宽取值的原因）。
fn bit_unpack(cpu: &mut Cpu, bus: &mut Bus) {
    let src = cpu.reg(0);
    let dst = cpu.reg(1);
    let info = cpu.reg(2);
    let source_len = u32::from(bus.read_u16(info));
    let source_width = u32::from(bus.read_u8(info + 2));
    let dest_width = u32::from(bus.read_u8(info + 3));
    let offset = bus.read_u32(info + 4);
    let offset_if_nonzero = offset & (1 << 31) != 0;
    let offset = offset & 0x7FFF_FFFF;
    // 目标位宽 < 8 的罕见形态（打包写）暂不支持；常用路径是 4→8 / 1→8
    if !matches!(source_width, 1 | 2 | 4 | 8) || !matches!(dest_width, 8 | 16 | 32) {
        return;
    }
    let units_per_byte = 8 / source_width;
    let mask = (1u32 << source_width) - 1;
    let mut dst_pos = dst;
    for src_pos in (src..).take(source_len as usize) {
        let byte = u32::from(bus.read_u8(src_pos));
        for unit in 0..units_per_byte {
            let value = (byte >> (unit * source_width)) & mask;
            // 偏移规则：bit31 = 1 → 只有非零源值加偏移
            let final_value = if offset_if_nonzero && value == 0 {
                value
            } else {
                value.wrapping_add(offset)
            };
            match dest_width {
                8 => bus.write_u8(dst_pos, final_value as u8),
                16 => bus.write_u16(dst_pos, final_value as u16),
                _ => bus.write_u32(dst_pos, final_value),
            }
            dst_pos += dest_width / 8;
        }
    }
}

/// LZ77 解压（SWI 0x11/0x12）。
///
/// 格式（GBATEK）：4 字节头 `0x10 | size<<8`；之后每字节是 8 个块的
/// 位图——位 1 = 压缩块（2 字节：`len-3 << 12 | disp-1`，从已解压数据
/// 回溯复制，这是 LZ 家族的核心）；位 0 = 原样块（1 字节计数 n，拷
/// n+1 字节）。`width=2` 是 VRAM 变体：目标按半字合成写（VRAM 只能
/// 16 位访问），这正是"为什么 LZ77UnCompVram 单独存在"的原因。
fn lz77(cpu: &mut Cpu, bus: &mut Bus, width: u32) {
    let mut source = cpu.reg(0);
    let mut dest = cpu.reg(1);
    let mut remaining = (bus.read_u32(source) >> 8) as usize; // 头的低 24 位 = 解压后大小
    source += 4;
    let mut blocks_remaining = 0u32;
    let mut blockheader = 0u32;
    let mut halfword = 0u16;
    while remaining > 0 {
        if blocks_remaining == 0 {
            blockheader = u32::from(bus.read_u8(source));
            source += 1;
            blocks_remaining = 8;
        }
        blocks_remaining -= 1;
        // 块标志从**最低位**取（GBATEK：LSB first）——这里曾按 MSB 写反，
        // 结果解压出的图全是雪花
        let compressed = blockheader & 1 != 0;
        blockheader >>= 1;
        if compressed {
            let block = u32::from(bus.read_u8(source)) | (u32::from(bus.read_u8(source + 1)) << 8);
            source += 2;
            let mut disp = dest.wrapping_sub((block & 0x0FFF) + 1);
            let mut bytes = (block >> 12) + 3;
            while bytes > 0 && remaining > 0 {
                bytes -= 1;
                remaining -= 1;
                let byte = read_back(bus, disp);
                if width == 2 {
                    halfword = vram_write_byte(bus, dest, byte, halfword);
                } else {
                    bus.write_u8(dest, byte);
                }
                disp = disp.wrapping_add(1);
                dest = dest.wrapping_add(1);
            }
        } else {
            let mut bytes = u32::from(bus.read_u8(source)) + 1;
            source += 1;
            while bytes > 0 && remaining > 0 {
                bytes -= 1;
                remaining -= 1;
                let byte = bus.read_u8(source);
                source += 1;
                if width == 2 {
                    halfword = vram_write_byte(bus, dest, byte, halfword);
                } else {
                    bus.write_u8(dest, byte);
                }
                dest = dest.wrapping_add(1);
            }
        }
    }
}

/// RL 解压（SWI 0x13/0x14）：行程编码，头 `0x30 | size<<8`。
/// 每块 1 字节：位 7 = 1 → 压缩（`(b&0x7F)+3` 次重复下一字节）；
/// 位 7 = 0 → 原样拷贝 `(b&0x7F)+1` 字节。
fn rl(cpu: &mut Cpu, bus: &mut Bus, width: u32) {
    let mut source = cpu.reg(0) & !3;
    let mut dest = cpu.reg(1);
    let mut remaining = (bus.read_u32(source) >> 8) as usize;
    source += 4;
    let mut halfword = 0u16;
    while remaining > 0 {
        let header = bus.read_u8(source);
        source += 1;
        if header & 0x80 != 0 {
            let byte = bus.read_u8(source);
            source += 1;
            let mut runs = u32::from(header & 0x7F) + 3;
            while runs > 0 && remaining > 0 {
                runs -= 1;
                remaining -= 1;
                if width == 2 {
                    halfword = vram_write_byte(bus, dest, byte, halfword);
                } else {
                    bus.write_u8(dest, byte);
                }
                dest = dest.wrapping_add(1);
            }
        } else {
            let mut bytes = u32::from(header & 0x7F) + 1;
            while bytes > 0 && remaining > 0 {
                bytes -= 1;
                remaining -= 1;
                let byte = bus.read_u8(source);
                source += 1;
                if width == 2 {
                    halfword = vram_write_byte(bus, dest, byte, halfword);
                } else {
                    bus.write_u8(dest, byte);
                }
                dest = dest.wrapping_add(1);
            }
        }
    }
}

/// Huffman 解压（SWI 0x15）：位流按树解码。头 `0x20 | size<<8 | bits`。
/// 树节点 1 字节：位 0-5 = 偏移、位 6 = 左终止、位 7 = 右终止
/// （布局与 mGBA / GBATEK 一致）。
fn huffman(cpu: &mut Cpu, bus: &mut Bus) {
    let base = cpu.reg(0) & !3; // 树按字对齐读取
    let mut dest = cpu.reg(1);
    let header = bus.read_u32(base);
    let mut remaining = (header >> 8) as usize;
    let bits = header & 0xF;
    if bits == 0 || 32 % bits != 0 || bits == 1 {
        return; // 真机不支持的位宽：直接放弃（mGBA 同款保守行为）
    }
    let tree_size = (u32::from(bus.read_u8(base + 4)) << 1) + 1;
    let tree_base = base + 5;
    let mut source = tree_base + tree_size;
    let mut node_ptr = tree_base;
    let mut value = 0u32;
    let mut value_bits = 0u32;
    while remaining > 0 {
        let bitstream = bus.read_u32(source);
        source += 4;
        for shift in (0..32).rev() {
            if remaining == 0 {
                break;
            }
            // 走树：位 1 = 右，位 0 = 左
            let go_right = (bitstream >> shift) & 1 == 1;
            let node = bus.read_u8(node_ptr);
            let next = (node_ptr & !1) + (u32::from(node & 0x3F) << 1) + 2;
            let (terminal, data_addr) = if go_right {
                (node & 0x80 != 0, next + 1)
            } else {
                (node & 0x40 != 0, next)
            };
            if !terminal {
                node_ptr = data_addr;
                continue;
            }
            // 到叶子：取出 bits 位数据
            let data = u32::from(bus.read_u8(data_addr));
            value = (value << bits) | data;
            value_bits += bits;
            if value_bits == 32 {
                bus.write_u32(dest, value); // Huffman 无 VRAM 变体，恒按字写
                dest += 4;
                value_bits = 0;
                remaining = remaining.saturating_sub(4);
            }
            node_ptr = tree_base; // 回到树根
        }
    }
    // 尾部不足 4 字节时按位宽写回剩余（保持与 mGBA 的容错一致：略）
}

/// 从已解压数据回溯读 1 字节（LZ 压缩块的引用源）。
fn read_back(bus: &mut Bus, addr: u32) -> u8 {
    // 回溯源是刚写出的数据，u8 读口不触发写路径的复制规则，语义正确
    bus.read_u8(addr)
}

/// 向 VRAM 写 1 字节的"半字合成"逻辑：VRAM 是 16 位总线，只有偶地址
/// 才有完整的半字可写。偶地址只**累积**进半字寄存器，奇地址到达时
/// 才把完整半字落盘。返回更新后的累加器。
fn vram_write_byte(bus: &mut Bus, dest: u32, byte: u8, halfword: u16) -> u16 {
    if dest & 1 == 0 {
        u16::from(byte) // 只累积，不写（低半）
    } else {
        bus.write_u16(dest - 1, halfword | (u16::from(byte) << 8));
        0
    }
}

/// MidiKey2Freq（SWI 0x1F）：m4a 引擎把 MIDI 音符号换算成采样步进。
/// r0 = WaveData 指针（+4 是采样率），r1 = MIDI 音高，r2 = 1/256 半音。
fn midi_key2freq(cpu: &mut Cpu, bus: &mut Bus) {
    let wave = cpu.reg(0);
    let key = bus.read_u32(wave + 4);
    let pitch = cpu.reg(1) as i32;
    let fine = cpu.reg(2) as i32;
    let semitones = (180.0 - pitch as f64 - fine as f64 / 256.0) / 12.0;
    cpu.regs[0] = (key as f64 / f64::powf(2.0, semitones)) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    fn setup() -> (Cpu, Bus) {
        (
            Cpu::new_no_bios(),
            Bus::new(Cartridge::from_bytes(vec![0; 0x100].into()).unwrap()),
        )
    }

    #[test]
    fn div_basic_and_zero() {
        let (mut cpu, _) = setup();
        div(&mut cpu, 7, 2);
        assert_eq!(cpu.reg(0), 3);
        assert_eq!(cpu.reg(1), 1);
        assert_eq!(cpu.reg(3), 3);
        div(&mut cpu, -7, 2);
        assert_eq!(cpu.reg(0) as i32, -3); // 向零取整
        assert_eq!(cpu.reg(1) as i32, -1);
        div(&mut cpu, 5, 0);
        assert_eq!(cpu.reg(3), 1); // 除零走"尽力而为"路径
    }

    #[test]
    fn isqrt_matches_floor() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(15), 3);
        assert_eq!(isqrt(16), 4);
        assert_eq!(isqrt(0xFFFF_FFFF), 65535);
    }

    #[test]
    fn cpu_set_copies_and_fills() {
        let (mut cpu, mut bus) = setup();
        for i in 0..4u32 {
            bus.write_u32(0x0200_0000 + i * 4, 0x1000 + i);
        }
        cpu.regs[0] = 0x0200_0000;
        cpu.regs[1] = 0x0200_1000;
        cpu.regs[2] = 4 | (1 << 26); // 4 个字（bit26=1 = 32 位宽度），拷贝模式
        cpu_set(&mut cpu, &mut bus, false);
        assert_eq!(bus.read_u32(0x0200_1000), 0x1000);
        assert_eq!(bus.read_u32(0x0200_100C), 0x1003);

        cpu.regs[0] = 0xDEAD_BEEF;
        cpu.regs[1] = 0x0200_2000;
        cpu.regs[2] = 2 | (1 << 24) | (1 << 26); // 2 个字 + 填充 + 32 位
        cpu_set(&mut cpu, &mut bus, false);
        assert_eq!(bus.read_u32(0x0200_2004), 0xDEAD_BEEF);
    }

    #[test]
    fn lz77_roundtrip() {
        let (mut cpu, mut bus) = setup();
        // 手工构造 LZ77 数据（GBATEK 位序：块标志 LSB first）：
        // 头 4 字节 = 0x10 | size<<8；位图 0b10 → 块1 原样、块2 压缩；
        // 解压结果应为 "ABABA"（5 字节）——压缩块从 2 字节前回溯复制 3 字节
        let src = 0x0200_0000;
        bus.write_u8(src, 0x10); // 签名
        bus.write_u8(src + 1, 0x05); // size = 5
        bus.write_u16(src + 2, 0);
        bus.write_u8(src + 4, 0b10); // 位图：bit0=原样，bit1=压缩
        bus.write_u8(src + 5, 0x01); // 原样块：n=1 → 拷 n+1=2 字节
        bus.write_u8(src + 6, b'A'); // 原样数据
        bus.write_u8(src + 7, b'B');
        // 压缩块：高 4 位 len-3=0 → 复制 3 字节；低 12 位 disp-1=1 → 回溯 2 字节
        bus.write_u16(src + 8, 0x0001);
        cpu.regs[0] = src;
        cpu.regs[1] = 0x0200_1000;
        lz77(&mut cpu, &mut bus, 1);
        assert_eq!(bus.read_u8(0x0200_1000), b'A');
        assert_eq!(bus.read_u8(0x0200_1001), b'B');
        assert_eq!(bus.read_u8(0x0200_1002), b'A');
        assert_eq!(bus.read_u8(0x0200_1003), b'B');
        assert_eq!(bus.read_u8(0x0200_1004), b'A');
    }

    #[test]
    fn rl_roundtrip() {
        let (mut cpu, mut bus) = setup();
        // 头：0x30 | size<<8；压缩块 0x82（重复 5 次）+ 值 0xAA
        let src = 0x0200_0000;
        bus.write_u8(src, 0x30);
        bus.write_u8(src + 1, 0x05);
        bus.write_u8(src + 4, 0x82);
        bus.write_u8(src + 5, 0xAA);
        cpu.regs[0] = src;
        cpu.regs[1] = 0x0200_1000;
        rl(&mut cpu, &mut bus, 1);
        for i in 0..5u32 {
            assert_eq!(bus.read_u8(0x0200_1000 + i), 0xAA);
        }
    }

    #[test]
    fn midi_key2freq_basic() {
        let (mut cpu, mut bus) = setup();
        bus.write_u32(0x0200_0004, 0x1000); // WaveData.freq
        cpu.regs[0] = 0x0200_0000;
        cpu.regs[1] = 180; // 音高 180 → semitones = 0 → freq 原样
        cpu.regs[2] = 0;
        midi_key2freq(&mut cpu, &mut bus);
        assert_eq!(cpu.reg(0), 0x1000);
    }
}
