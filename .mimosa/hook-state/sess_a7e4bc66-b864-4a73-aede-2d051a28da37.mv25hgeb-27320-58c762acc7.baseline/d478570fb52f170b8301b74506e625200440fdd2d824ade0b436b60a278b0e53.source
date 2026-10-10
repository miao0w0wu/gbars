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

    /// 推进一帧：CPU 执行指令直到耗尽 280,896 周期，沿途按扫描线边界渲染、
    /// 推进定时器。
    pub fn run_frame(&mut self, cpu: &mut Cpu, bus: &mut Bus) -> Result<(), CpuError> {
        while self.now < CYCLES_PER_FRAME {
            cpu.step(bus)?;
            let cycles = cpu.take_cycles().max(1);
            let after = self.now + cycles;
            // 定时器推进到当前点（行事件只影响 PPU 状态，不影响定时器）；
            // 溢出按 TMxCNT_H 位 6 的使能投递 IE 位 3-6。
            let timer_overflow = bus.io.timers.tick(cycles);
            for i in 0..4usize {
                if timer_overflow & (1 << i) != 0 && bus.io.timers.timer[i].irq_enabled() {
                    bus.io.request_irq(1 << (3 + i));
                }
            }
            bus.io.tick(cycles);
            self.advance(bus, after);
            self.now = after;
        }
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
            } else {
                ppu.set_dispstat_flag(0, true); // VBlank
                self.vblank_reached = true;
                if ppu.dispstat() & (1 << 3) != 0 {
                    bus.io.request_irq(1 << 0);
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
