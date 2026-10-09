//! 内存总线：CPU 与所有硬件之间的"主干道"。
//!
//! # 硬件背景：GBA 的内存地图
//!
//! ARM7TDMI 有 32 位地址空间，但 GBA 只在几个固定窗口挂了存储设备，
//! 且各设备的总线宽度不同（这直接影响访问时序与怪癖行为）：
//!
//! | 区域 | 起始地址 | 大小 | 总线宽 | 备注 |
//! |------|----------|------|--------|------|
//! | BIOS | 0x00000000 | 16 KiB | 16 位 | 本项目无真 BIOS，暂为全 0 |
//! | EWRAM | 0x02000000 | 256 KiB | 16 位 | 板载慢速 RAM |
//! | IWRAM | 0x03000000 | 32 KiB | 32 位 | 片内快速 RAM，游戏热代码放这里 |
//! | I/O | 0x04000000 | 1 KiB | — | 硬件寄存器（各里程碑逐个实现） |
//! | 调色板 | 0x05000000 | 1 KiB | 16 位 | BG/OBJ 各 256 色 |
//! | VRAM | 0x06000000 | 96 KiB | 16 位 | 64K 背景 + 32K 精灵 |
//! | OAM | 0x07000000 | 1 KiB | 32 位 | 128 个精灵属性 |
//! | ROM | 0x08000000 | ≤32 MiB | 16 位 | 卡带（含等待状态，M2 实现时序） |
//! | 存档 | 0x0E000000 | ≤64 KiB | **8 位** | SRAM/Flash（M3） |
//!
//! # 三个必知的硬件"怪癖"（每个都有单元测试锁住）
//!
//! 1. **地址镜像**：区域实际容量不足窗口大小时低位回卷——访问 0x02040000
//!    会命中 EWRAM 的 0x000000。实现 = 相对地址 `& (容量-1)`；
//! 2. **VRAM/调色板的 8 位写**：16 位总线收到 8 位数据时，字节被**复制进
//!    半字的两个字节**（仅背景区；精灵区是普通单字节写——见 GBATEK
//!    "VRAM" 注）。处理错了游戏画面会花屏；
//! 3. **OAM 忽略 8 位写**（32 位总线设备）；SRAM 是 8 位总线，读半字/字时
//!    同一字节在所有字节通道上重复出现。
//!
//! I/O 区暂时只是原始字节数组——寄存器语义（DISPCNT、TM0CNT……）由后续
//! 里程碑的各硬件模块接管。"从 BIOS 之外读 BIOS 应返回开放总线值"的规则
//! 也留到 M2 时序完善时补齐（当前返回镜像内的 0）。

use crate::cartridge::Cartridge;

pub const BIOS_SIZE: usize = 0x004000;
pub const EWRAM_SIZE: usize = 0x040000;
pub const IWRAM_SIZE: usize = 0x008000;
pub const IO_SIZE: usize = 0x000400;
pub const PALETTE_SIZE: usize = 0x000400;
pub const VRAM_SIZE: usize = 0x018000;
pub const OAM_SIZE: usize = 0x000400;
pub const SRAM_SIZE: usize = 0x010000;

// 容量都是 2 的幂，"容量-1"就是镜像掩码：相对地址 & 掩码 = 区域内偏移
const EWRAM_MASK: usize = EWRAM_SIZE - 1;
const IWRAM_MASK: usize = IWRAM_SIZE - 1;
const IO_MASK: usize = IO_SIZE - 1;
const PALETTE_MASK: usize = PALETTE_SIZE - 1;
const OAM_MASK: usize = OAM_SIZE - 1;
const SRAM_MASK: usize = SRAM_SIZE - 1;
/// ROM 窗口 0x08000000–0x09FFFFFF 共 32 MiB（0x0A/0x0C 区域是它的镜像）
const ROM_WINDOW_MASK: u32 = 0x01FF_FFFF;

pub struct Bus {
    bios: Box<[u8]>,
    ewram: Box<[u8]>,
    iwram: Box<[u8]>,
    io: Box<[u8]>,
    palette: Box<[u8]>,
    vram: Box<[u8]>,
    oam: Box<[u8]>,
    sram: Box<[u8]>,
    cart: Cartridge,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        Self {
            bios: vec![0; BIOS_SIZE].into(),
            ewram: vec![0; EWRAM_SIZE].into(),
            iwram: vec![0; IWRAM_SIZE].into(),
            io: vec![0; IO_SIZE].into(),
            palette: vec![0; PALETTE_SIZE].into(),
            vram: vec![0; VRAM_SIZE].into(),
            oam: vec![0; OAM_SIZE].into(),
            sram: vec![0; SRAM_SIZE].into(),
            cart,
        }
    }

    pub fn cart(&self) -> &Cartridge {
        &self.cart
    }

    /// VRAM 相对索引：128K 窗口内只有前 96K 真实存在，
    /// 96K 以上的"幻影区"按硬件走线别名到精灵区（见 GBATEK VRAM 注）。
    fn vram_index(addr: u32) -> usize {
        let rel = (addr & 0x0001_FFFF) as usize;
        if rel >= VRAM_SIZE {
            rel - 0x008000
        } else {
            rel
        }
    }

    pub fn read_u8(&mut self, addr: u32) -> u8 {
        match addr >> 24 {
            0x00 | 0x01 => self.bios[(addr as usize) & (BIOS_SIZE - 1)],
            0x02 => self.ewram[(addr as usize) & EWRAM_MASK],
            0x03 => self.iwram[(addr as usize) & IWRAM_MASK],
            0x04 => self.io[(addr as usize) & IO_MASK],
            0x05 => self.palette[(addr as usize) & PALETTE_MASK],
            0x06 => self.vram[Self::vram_index(addr)],
            0x07 => self.oam[(addr as usize) & OAM_MASK],
            0x08..=0x0D => {
                // ROM 窗口（三段等待状态共用一块 32 MiB 空间）。
                // 超出 ROM 实际大小的读：真机返回"开放总线"垃圾值，
                // 这里简化为 0（M3 做 EEPROM 时再细化）。
                let rel = (addr & ROM_WINDOW_MASK) as usize;
                let rom = self.cart.rom();
                if rel < rom.len() { rom[rel] } else { 0 }
            }
            0x0E | 0x0F => self.sram[(addr as usize) & SRAM_MASK],
            _ => 0, // 未映射区域（0x10000000 以上等）
        }
    }

    pub fn read_u16(&mut self, addr: u32) -> u16 {
        let a = addr & !1; // 16 位访问强制对齐到半字
        // SRAM 挂在 8 位总线上：16 位读 = 同一字节在两个字节通道重复出现
        if matches!(a >> 24, 0x0E | 0x0F) {
            let b = self.read_u8(a);
            return u16::from(b) * 0x0101;
        }
        let lo = self.read_u8(a);
        let hi = self.read_u8(a + 1);
        u16::from(lo) | (u16::from(hi) << 8)
    }

    pub fn read_u32(&mut self, addr: u32) -> u32 {
        let a = addr & !3; // 32 位访问强制对齐到字
        if matches!(a >> 24, 0x0E | 0x0F) {
            let b = self.read_u8(a);
            return u32::from(b) * 0x0101_0101;
        }
        let b0 = self.read_u8(a);
        let b1 = self.read_u8(a + 1);
        let b2 = self.read_u8(a + 2);
        let b3 = self.read_u8(a + 3);
        u32::from(b0) | (u32::from(b1) << 8) | (u32::from(b2) << 16) | (u32::from(b3) << 24)
    }

    pub fn write_u8(&mut self, addr: u32, value: u8) {
        self.store_u8(addr, value, true);
    }

    pub fn write_u16(&mut self, addr: u32, value: u16) {
        let a = addr & !1;
        self.store_u8(a, value as u8, false);
        self.store_u8(a + 1, (value >> 8) as u8, false);
    }

    pub fn write_u32(&mut self, addr: u32, value: u32) {
        let a = addr & !3;
        for i in 0..4 {
            self.store_u8(a + i, (value >> (i * 8)) as u8, false);
        }
    }

    /// 按区域分发一个字节写入。
    ///
    /// `byte_access` 标记这是否为一次"原生 8 位写"：只有原生 8 位写才触发
    /// 调色板/VRAM 背景区的字节复制与 OAM 忽略规则；16/32 位写拆开的字节
    /// 走普通路径（它们本来就是完整半字/字写的一部分）。
    fn store_u8(&mut self, addr: u32, b: u8, byte_access: bool) {
        match addr >> 24 {
            0x02 => self.ewram[(addr as usize) & EWRAM_MASK] = b,
            0x03 => self.iwram[(addr as usize) & IWRAM_MASK] = b,
            0x04 => self.io[(addr as usize) & IO_MASK] = b,
            0x05 => {
                if byte_access {
                    let i = ((addr as usize) & PALETTE_MASK) & !1;
                    self.palette[i] = b;
                    self.palette[i + 1] = b;
                } else {
                    self.palette[(addr as usize) & PALETTE_MASK] = b;
                }
            }
            0x06 => {
                let i = Self::vram_index(addr);
                if byte_access && i < 0x010000 {
                    // 背景区：8 位写复制进整个半字（总线 16 位宽的物理结果）
                    let j = i & !1;
                    self.vram[j] = b;
                    self.vram[j + 1] = b;
                } else {
                    // 精灵区（≥0x010000）：普通 8 位写
                    self.vram[i] = b;
                }
            }
            0x07 => {
                if !byte_access {
                    self.oam[(addr as usize) & OAM_MASK] = b;
                }
                // 原生 8 位写被 OAM 忽略（32 位总线设备）
            }
            0x0E | 0x0F => self.sram[(addr as usize) & SRAM_MASK] = b,
            _ => {} // BIOS/ROM 只读；未映射区域写入丢弃
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    /// 64 字节迷你"ROM"：内容 = 自身偏移量，方便验证 ROM 窗口读数。
    fn bus() -> Bus {
        let mut rom = vec![0u8; 0x100];
        for (i, b) in rom.iter_mut().enumerate() {
            *b = i as u8;
        }
        Bus::new(Cartridge::from_bytes(rom.into()).unwrap())
    }

    #[test]
    fn ram_roundtrip_all_widths() {
        let mut b = bus();
        b.write_u32(0x0200_0100, 0xDEAD_BEEF);
        b.write_u16(0x0300_0200, 0x1234);
        b.write_u8(0x0200_0004, 0x7F);
        assert_eq!(b.read_u32(0x0200_0100), 0xDEAD_BEEF);
        assert_eq!(b.read_u16(0x0300_0200), 0x1234);
        assert_eq!(b.read_u8(0x0200_0004), 0x7F);
    }

    #[test]
    fn mirrors_and_rom_window() {
        let mut b = bus();
        b.write_u32(0x0200_0100, 0xABCD_EF01);
        assert_eq!(b.read_u32(0x0204_0100), 0xABCD_EF01); // EWRAM 镜像
        b.write_u16(0x0300_0004, 0x1234);
        assert_eq!(b.read_u16(0x0300_8004), 0x1234); // IWRAM 镜像
        // ROM：镜像按 (地址 - 0x08000000) & 0x01FFFFFF 计算——
        // 0x0A 区域是同一 32 MiB 窗口的镜像，偏移回卷到窗口开头
        assert_eq!(b.read_u32(0x0800_0008), 0x0B0A_0908);
        assert_eq!(b.read_u32(0x0A00_0008), 0x0B0A_0908);
        // 主窗口后半段（0x0900_0008 → 窗口内偏移 0x01000008）：
        // 小测试 ROM 没有这个位置的数据 → 开放总线桩返回 0
        assert_eq!(b.read_u32(0x0900_0008), 0);
    }

    #[test]
    fn vram_phantom_region_aliases_to_obj() {
        let mut b = bus();
        b.write_u32(0x0601_0000, 0x1122_3344);
        assert_eq!(b.read_u32(0x0601_8000), 0x1122_3344);
    }

    #[test]
    fn vram_byte_write_duplicates_in_bg_region() {
        let mut b = bus();
        b.write_u8(0x0600_0000, 0xAB);
        assert_eq!(b.read_u16(0x0600_0000), 0xABAB);
    }

    #[test]
    fn vram_byte_write_is_plain_in_obj_region() {
        let mut b = bus();
        b.write_u16(0x0601_0002, 0xFF00); // 先放底色
        b.write_u8(0x0601_0002, 0x7E); // OBJ 区 8 位写只动一个字节
        assert_eq!(b.read_u16(0x0601_0002), 0xFF7E);
    }

    #[test]
    fn palette_byte_write_duplicates() {
        let mut b = bus();
        b.write_u8(0x0500_0004, 0x3C);
        assert_eq!(b.read_u16(0x0500_0004), 0x3C3C);
    }

    #[test]
    fn oam_ignores_byte_writes() {
        let mut b = bus();
        b.write_u16(0x0700_0000, 0x00FF);
        b.write_u8(0x0700_0000, 0xA5);
        assert_eq!(b.read_u16(0x0700_0000), 0x00FF);
    }

    #[test]
    fn sram_byte_repeats_on_wide_reads() {
        let mut b = bus();
        b.write_u8(0x0E00_0000, 0x5A);
        assert_eq!(b.read_u16(0x0E00_0000), 0x5A5A);
        assert_eq!(b.read_u32(0x0E00_0000), 0x5A5A_5A5A);
    }
}
