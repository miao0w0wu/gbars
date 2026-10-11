//! 统一时钟与扫描线调度。

use crate::bus::Bus;
use crate::cpu::{Cpu, CpuError};
use crate::ppu::{CYCLES_PER_LINE, TOTAL_LINES, VISIBLE_LINES};

/// 一帧的总周期数：228 线 × 1232 周期。
pub const CYCLES_PER_FRAME: u32 = TOTAL_LINES as u32 * CYCLES_PER_LINE;

pub struct Scheduler {
    /// 当前帧内已消耗的周期（0 .. CYCLES_PER_FRAME）。
    now: u32,
    /// 下一条待处理的扫描线（VCOUNT 值）。
    next_line: u16,
    /// 本帧是否已到达 VBlank。
    pub vblank_reached: bool,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            now: 0,
            next_line: 0,
            vblank_reached: false,
        }
    }

    pub fn reset_frame(&mut self) {
        self.now = 0;
        self.next_line = 0;
        self.vblank_reached = false;
    }

    pub fn now(&self) -> u32 {
        self.now
    }

    /// 推进一帧：CPU 执行指令直到耗尽 280,896 周期，沿途按扫描线边界渲染、
    /// 推进定时器。
    pub fn run_frame(&mut self, cpu: &mut Cpu, bus: &mut Bus) -> Result<(), CpuError> {
        while self.now < CYCLES_PER_FRAME {
            self.run_instruction(cpu, bus)?;
        }
        Ok(())
    }

    pub fn run_instruction(&mut self, cpu: &mut Cpu, bus: &mut Bus) -> Result<(), CpuError> {
        cpu.step(bus)?;
        let cycles = cpu.take_cycles().max(1);
        let after = self.now + cycles;
        let timer_overflow = bus.io.timers.tick(cycles);
        for i in 0..4usize {
            if timer_overflow & (1 << i) != 0 && bus.io.timers.timer[i].irq_enabled() {
                bus.io.request_irq(1 << (3 + i));
            }
        }
        bus.io.tick(cycles);
        self.advance(bus, after);
        self.now = after;
        Ok(())
    }

    /// 处理 [now, after) 区间内到期的扫描线。
    ///
    /// DISPSTAT 位 0 = VBlank、位 1 = HBlank、位 3 = VBlank 中断使能、
    /// 位 4 = HBlank 中断使能；IF 由对应事件置起（位 0 = VBlank 中断）。
    fn advance(&mut self, bus: &mut Bus, after: u32) {
        while self.next_line < TOTAL_LINES {
            let line = self.next_line;
            let line_end = (line as u32 + 1) * CYCLES_PER_LINE;
            if line_end > after {
                break;
            }
            let ppu = bus.ppu_mut();
            ppu.set_vcount(line);
            let line_started = line_end - CYCLES_PER_LINE;
            if line < VISIBLE_LINES {
                ppu.render_line(line);
                ppu.set_dispstat_flag(0, false);
                // HDraw 结束进 HBlank（此实现按整行事件近似：行尾置 HBlank）
                ppu.set_dispstat_flag(1, true);
                if ppu.dispstat() & (1 << 4) != 0 {
                    bus.io.request_irq(1 << 1);
                }
                bus.run_dma_triggers(2);
            } else {
                ppu.set_dispstat_flag(0, true); // VBlank
                self.vblank_reached = true;
                if ppu.dispstat() & (1 << 3) != 0 {
                    bus.io.request_irq(1 << 0);
                }
                if line == VISIBLE_LINES {
                    bus.run_dma_triggers(1);
                }
            }
            let _ = line_started;
            self.next_line += 1;
        }
    }

    /// 当前 VCOUNT（供 headless 输出）。
    pub fn vcount(&self) -> u16 {
        self.next_line.saturating_sub(1).min(TOTAL_LINES - 1)
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    fn spin() -> (Cpu, Bus) {
        let mut rom = vec![0u8; 0x100];
        rom[0..4].copy_from_slice(&0xEAFF_FFFEu32.to_le_bytes());
        let bus = Bus::new(Cartridge::from_bytes(rom.into()).unwrap());
        (Cpu::new_no_bios(), bus)
    }

    #[test]
    fn full_frame_reaches_line_227_with_vblank_flags() {
        let (mut cpu, mut bus) = spin();
        let mut s = Scheduler::new();
        s.run_frame(&mut cpu, &mut bus).unwrap();
        assert_eq!(s.vcount(), 227);
        assert!(s.vblank_reached);
        assert_eq!(bus.ppu().dispstat() & 0x3, 0x3);
    }

    #[test]
    fn vblank_flag_toggles_at_line_160() {
        let (mut cpu, mut bus) = spin();
        let mut s = Scheduler::new();
        while bus.ppu().vcount() < 159 {
            s.run_instruction(&mut cpu, &mut bus).unwrap();
        }
        assert_eq!(bus.ppu().dispstat() & 1, 0);
        assert_eq!(bus.ppu().dispstat() & 2, 2);
        while bus.ppu().vcount() < 160 {
            s.run_instruction(&mut cpu, &mut bus).unwrap();
        }
        assert_eq!(bus.ppu().dispstat() & 1, 1);
    }

    #[test]
    fn vblank_irq_requested_only_when_dispstat_enables() {
        let (mut cpu, mut bus) = spin();
        bus.write_u16(0x0400_0004, 0x0008);
        let mut s = Scheduler::new();
        s.run_frame(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.io.read_u16(0x0202, 0) & 0x1, 0x1);

        let (mut cpu, mut bus) = spin();
        let mut s = Scheduler::new();
        s.run_frame(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.io.read_u16(0x0202, 0) & 0x1, 0x0);
    }

    #[test]
    fn hblank_irq_requested_when_enabled() {
        let (mut cpu, mut bus) = spin();
        bus.write_u16(0x0400_0004, 0x0010);
        let mut s = Scheduler::new();
        s.run_frame(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.io.read_u16(0x0202, 0) & 0x2, 0x2);
    }

    #[test]
    fn timer_overflow_requests_irq_bit3() {
        let (mut cpu, mut bus) = spin();
        bus.write_u16(0x0400_0100, 0x0000);
        bus.write_u16(0x0400_0102, 0x00C0);
        let mut s = Scheduler::new();
        s.run_frame(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.io.read_u16(0x0202, 0) & 0x8, 0x8);
    }

    #[test]
    fn hblank_dma_runs_once_per_visible_line() {
        let (mut cpu, mut bus) = spin();
        for i in 0..192u32 {
            bus.write_u16(0x0200_0000 + i * 2, 0x5000 + i as u16);
        }
        bus.write_u32(0x0400_00D4, 0x0200_0000);
        bus.write_u32(0x0400_00D8, 0x0200_0200);
        bus.write_u16(0x0400_00DC, 1);
        bus.write_u16(0x0400_00DE, 0xA200);
        let mut s = Scheduler::new();
        s.run_frame(&mut cpu, &mut bus).unwrap();
        for j in [0u32, 1, 80, 159] {
            assert_eq!(
                bus.read_u16(0x0200_0200 + j * 2),
                0x5000 + j as u16,
                "dst[{j}]"
            );
        }
        assert_eq!(bus.read_u16(0x0200_0200 + 160 * 2), 0);
    }

    #[test]
    fn vblank_dma_fires_at_line_160() {
        let (mut cpu, mut bus) = spin();
        bus.write_u16(0x0200_0000, 0x1234);
        bus.write_u32(0x0400_00D4, 0x0200_0000);
        bus.write_u32(0x0400_00D8, 0x0200_0100);
        bus.write_u16(0x0400_00DC, 1);
        bus.write_u16(0x0400_00DE, 0x9000);
        let mut s = Scheduler::new();
        s.run_frame(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x0200_0100), 0x1234);
        assert_eq!(bus.io.dma.channel[3].ctrl & 0x8000, 0);
    }
}
