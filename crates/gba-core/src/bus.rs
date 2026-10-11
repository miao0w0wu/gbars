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
//! | I/O | 0x04000000 | 1 KiB | — | 硬件寄存器（本里程碑由 PPU 接管图形部分） |
//! | 调色板 | 0x05000000 | 1 KiB | 16 位 | BG/OBJ 各 256 色 |
//! | VRAM | 0x06000000 | 96 KiB | 16 位 | 64K 背景 + 32K 精灵 |
//! | OAM | 0x07000000 | 1 KiB | 32 位 | 128 个精灵属性 |
//! | ROM | 0x08000000 | ≤32 MiB | 16 位 | 卡带（含等待状态） |
//! | 存档 | 0x0E000000 | ≤64 KiB | **8 位** | SRAM/Flash（M3） |
//!
//! 调色板/VRAM/OAM 三块显存由 [`crate::ppu::Ppu`] 持有（渲染器要直接读它们，
//! 再走总线会绕远），本模块负责地址译码、镜像与位宽怪癖后转发。

use crate::cartridge::Cartridge;
use crate::dma::Dma;
use crate::io::Io;

pub const BIOS_SIZE: usize = 0x004000;
pub const EWRAM_SIZE: usize = 0x040000;
pub const IWRAM_SIZE: usize = 0x008000;
pub const PALETTE_SIZE: usize = 0x000400;
pub const VRAM_SIZE: usize = 0x018000;
pub const OAM_SIZE: usize = 0x000400;
pub const SRAM_SIZE: usize = 0x010000;

const EWRAM_MASK: usize = EWRAM_SIZE - 1;
const IWRAM_MASK: usize = IWRAM_SIZE - 1;
const PALETTE_MASK: usize = PALETTE_SIZE - 1;
const OAM_MASK: usize = OAM_SIZE - 1;
const SRAM_MASK: usize = SRAM_SIZE - 1;
/// ROM 窗口 0x08000000–0x09FFFFFF 共 32 MiB（0x0A/0x0C 区域是它的镜像）
const ROM_WINDOW_MASK: u32 = 0x01FF_FFFF;

pub struct Bus {
    bios: Box<[u8]>,
    ewram: Box<[u8]>,
    iwram: Box<[u8]>,
    sram: Box<[u8]>,
    cart: Cartridge,
    pub io: Io,
    /// 指令执行期间累计的数据访问周期；CPU 每步取走后计入全局时钟。
    data_cycles: u32,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        let mut bus = Self {
            bios: vec![0; BIOS_SIZE].into(),
            ewram: vec![0; EWRAM_SIZE].into(),
            iwram: vec![0; IWRAM_SIZE].into(),
            sram: vec![0; SRAM_SIZE].into(),
            cart,
            io: Io::new(),
            data_cycles: 0,
        };
        bus.install_irq_stub();
        bus
    }

    /// 在 BIOS 内存写入 mGBA hle-bios.s irqBase 的机器码（翻译为静态字节），
    /// 让"无 BIOS"也能执行 IRQ 入口：保存寄存器 → 调用游戏 handler（读
    /// 0x03FFFFFC）→ 恢复 → SUBS PC,LR,#4 返回。
    fn install_irq_stub(&mut self) {
        let put = |bios: &mut [u8], addr: u32, word: u32| {
            let i = addr as usize;
            bios[i..i + 4].copy_from_slice(&word.to_le_bytes());
        };
        // 0x18 向量：b 0x120（与 mGBA hle-bios.s 的 `b irqBase` 一致）
        put(&mut self.bios, 0x18, 0xEA00_0040);
        // irqBase（mGBA hle-bios.s 的 IRQ 入口，逐条翻译）
        put(&mut self.bios, 0x120, 0xE92D_500F); // STMFD SP!, {r0-r3,r12,lr}
        put(&mut self.bios, 0x124, 0xE3A0_0404); // MOV r0, #0x04000000
        put(&mut self.bios, 0x128, 0xE28F_E000); // ADD lr, pc, #0
        put(&mut self.bios, 0x12C, 0xE510_F004); // LDR pc, [r0, #-4] → [0x03FFFFFC]
        put(&mut self.bios, 0x130, 0xE8BD_500F); // LDMFD SP!, {r0-r3,r12,lr}
        put(&mut self.bios, 0x134, 0xE25E_F004); // SUBS pc, lr, #4
    }

    pub fn cart(&self) -> &Cartridge {
        &self.cart
    }

    pub fn ppu(&self) -> &crate::ppu::Ppu {
        &self.io.ppu
    }

    pub fn ppu_mut(&mut self) -> &mut crate::ppu::Ppu {
        &mut self.io.ppu
    }

    /// 取走并清零本步累计的数据访问周期。
    pub(crate) fn take_data_cycles(&mut self) -> u32 {
        core::mem::take(&mut self.data_cycles)
    }

    fn charge_access(&mut self, addr: u32, word: bool) {
        self.data_cycles += Self::access_cycles(addr, word);
    }

    /// 一次数据访问的基础周期（不含等待状态）。
    ///
    /// GBA 各区域的取指/数据总线宽度不同（见模块文档的内存表），
    /// 这让"同样一条指令"在不同区域执行速度不同：
    ///
    /// | 区域 | 宽度 | 16 位访问 | 32 位访问 |
    /// |------|------|-----------|-----------|
    /// | IWRAM / I/O / OAM | 32 位 | 1 | 1 |
    /// | EWRAM / 调色板 / VRAM / ROM | 16 位 | 1 | 2（拆成两次半字） |
    pub fn access_cycles(addr: u32, word: bool) -> u32 {
        match addr >> 24 {
            0x02 => {
                if word {
                    2
                } else {
                    1
                }
            }
            0x08..=0x0D => {
                if word {
                    4
                } else {
                    2
                }
            }
            _ => 1,
        }
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
        self.charge_access(addr, false);
        self.read_u8_raw(addr)
    }

    pub fn read_u16(&mut self, addr: u32) -> u16 {
        self.charge_access(addr, false);
        self.read_u16_raw(addr)
    }

    pub fn read_u32(&mut self, addr: u32) -> u32 {
        self.charge_access(addr, true);
        self.read_u32_raw(addr)
    }

    /// 供取指使用的无记账读：取指周期由 CPU 显式按宽度收取。
    pub(crate) fn fetch_u16(&mut self, addr: u32) -> u16 {
        self.read_u16_raw(addr)
    }

    pub(crate) fn fetch_u32(&mut self, addr: u32) -> u32 {
        self.read_u32_raw(addr)
    }

    fn read_u8_raw(&mut self, addr: u32) -> u8 {
        match addr >> 24 {
            0x00 | 0x01 => self.bios[(addr as usize) & (BIOS_SIZE - 1)],
            0x02 => self.ewram[(addr as usize) & EWRAM_MASK],
            0x03 => self.iwram[(addr as usize) & IWRAM_MASK],
            0x04 => self.io.read_u8(addr & 0x3FF),
            0x05 => self.io.ppu.palette[(addr as usize) & PALETTE_MASK],
            0x06 => self.io.ppu.vram[Self::vram_index(addr)],
            0x07 => self.io.ppu.oam[(addr as usize) & OAM_MASK],
            0x08..=0x0D => {
                let rel = (addr & ROM_WINDOW_MASK) as usize;
                let rom = self.cart.rom();
                if rel < rom.len() { rom[rel] } else { 0 }
            }
            0x0E | 0x0F => self.sram[(addr as usize) & SRAM_MASK],
            _ => 0,
        }
    }

    fn read_u16_raw(&mut self, addr: u32) -> u16 {
        let a = addr & !1; // 16 位访问强制对齐到半字
        // SRAM 挂在 8 位总线上：16 位读 = 同一字节在两个字节通道重复出现
        if matches!(a >> 24, 0x0E | 0x0F) {
            let b = self.read_u8_raw(a);
            return u16::from(b) * 0x0101;
        }
        let lo = self.read_u8_raw(a);
        let hi = self.read_u8_raw(a + 1);
        u16::from(lo) | (u16::from(hi) << 8)
    }

    fn read_u32_raw(&mut self, addr: u32) -> u32 {
        let a = addr & !3; // 32 位访问强制对齐到字
        if matches!(a >> 24, 0x0E | 0x0F) {
            let b = self.read_u8_raw(a);
            return u32::from(b) * 0x0101_0101;
        }
        let b0 = self.read_u8_raw(a);
        let b1 = self.read_u8_raw(a + 1);
        let b2 = self.read_u8_raw(a + 2);
        let b3 = self.read_u8_raw(a + 3);
        u32::from(b0) | (u32::from(b1) << 8) | (u32::from(b2) << 16) | (u32::from(b3) << 24)
    }

    pub fn write_u8(&mut self, addr: u32, value: u8) {
        self.charge_access(addr, false);
        self.store_u8(addr, value, true);
    }

    pub fn write_u16(&mut self, addr: u32, value: u16) {
        self.charge_access(addr, false);
        let a = addr & !1;
        self.store_u8(a, value as u8, false);
        self.store_u8(a + 1, (value >> 8) as u8, false);
    }

    pub fn write_u32(&mut self, addr: u32, value: u32) {
        self.charge_access(addr, true);
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
            0x04 => {
                self.io.write_u8(addr & 0x3FF, b);
                self.run_pending_dma();
            }
            0x05 => {
                if byte_access {
                    let i = ((addr as usize) & PALETTE_MASK) & !1;
                    self.io.ppu.palette[i] = b;
                    self.io.ppu.palette[i + 1] = b;
                } else {
                    self.io.ppu.palette[(addr as usize) & PALETTE_MASK] = b;
                }
            }
            0x06 => {
                let i = Self::vram_index(addr);
                if byte_access && i < 0x010000 {
                    // 背景区：8 位写复制进整个半字（总线 16 位宽的物理结果）
                    let j = i & !1;
                    self.io.ppu.vram[j] = b;
                    self.io.ppu.vram[j + 1] = b;
                } else {
                    // 精灵区（≥0x010000）：普通 8 位写
                    self.io.ppu.vram[i] = b;
                }
            }
            0x07 => {
                if !byte_access {
                    self.io.ppu.oam[(addr as usize) & OAM_MASK] = b;
                }
                // 原生 8 位写被 OAM 忽略（32 位总线设备）
            }
            0x0E | 0x0F => self.sram[(addr as usize) & SRAM_MASK] = b,
            _ => {}
        }
    }

    /// 执行所有标记为待跑的立即 DMA 传输。
    ///
    /// Io::write_u16 的 DMA 控制字写路径把通道号暂存到 `pending_dma`，
    /// 这里统一执行（避免在 &mut self.io 的借用里再借 bus）。
    fn run_pending_dma(&mut self) {
        while let Some(ch) = self.io.dma.take_pending() {
            let mut ch_state = self.io.dma.channel[ch];
            Dma::run_channel(ch, &mut ch_state, self);
            if ch_state.ctrl & 0x4000 != 0 {
                self.io.request_irq(1 << (8 + ch as u16));
            }
            self.io.dma.channel[ch] = ch_state;
        }
    }

    /// 执行触发码匹配 `timing`（1 = VBlank，2 = HBlank）的已使能通道。
    pub(crate) fn run_dma_triggers(&mut self, timing: u16) {
        for ch in 0..4usize {
            let mut ch_state = self.io.dma.channel[ch];
            if ch_state.ctrl & 0x8000 != 0 && (ch_state.ctrl >> 12) & 0x3 == timing {
                Dma::run_channel(ch, &mut ch_state, self);
                if ch_state.ctrl & 0x4000 != 0 {
                    self.io.request_irq(1 << (8 + ch as u16));
                }
                self.io.dma.channel[ch] = ch_state;
            }
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new(Cartridge::from_bytes(vec![0u8; 0xC0].into()).unwrap())
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
        assert_eq!(b.read_u32(0x0800_0008), 0x0B0A_0908);
        assert_eq!(b.read_u32(0x0A00_0008), 0x0B0A_0908);
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
        b.write_u16(0x0601_0002, 0xFF00);
        b.write_u8(0x0601_0002, 0x7E);
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
