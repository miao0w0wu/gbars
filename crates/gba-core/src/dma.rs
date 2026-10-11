//! DMA：4 通道拷贝引擎（立即触发 + VBlank/HBlank/Special 触发）。

/// 单通道可编程状态（寄存器镜像）。
#[derive(Clone, Copy, Default)]
pub struct Channel {
    pub src: u32,
    pub dst: u32,
    /// 使能边沿时刻的 dst 寄存器值（目标步进模式 3 的重载基址）。
    pub dst_base: u32,
    pub count: u16,
    /// DMAxCNT_H（0x040000B8+0xA）：
    /// 位 5-6 目标步进，位 7-8 源步进，位 9 重复，位 10 半字/字，
    /// 位 12-13 触发，位 14 完成中断，位 15 使能。
    pub ctrl: u16,
}

/// 地址步进增量（半字单位数）。
fn delta(bits: u16) -> i32 {
    match bits {
        0 => 1,
        1 => -1,
        2 => 0,
        _ => 1, // 3 = 递增重载（仅 DMA 目标，M3 处理；这里当递增）
    }
}

#[derive(Clone, Copy, Default)]
pub struct Dma {
    pub channel: [Channel; 4],
    /// write_ctrl 判定"需要立即传输"时暂存的通道号，由 Bus 取走执行。
    pending: Option<usize>,
}

impl Dma {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn take_pending(&mut self) -> Option<usize> {
        self.pending.take()
    }

    pub fn enabled(&self, ch: usize) -> bool {
        self.channel[ch].ctrl & 0x8000 != 0
    }

    /// 写控制字。使能位 0→1 边沿装载内部地址基址；触发码 0（立即）时
    /// 暂存通道号，由 Bus 的写路径统一执行。
    pub fn write_ctrl(&mut self, ch: usize, v: u16) {
        let was = self.enabled(ch);
        self.channel[ch].ctrl = v;
        if !was && self.enabled(ch) {
            self.channel[ch].dst_base = self.channel[ch].dst;
            if (v >> 12) & 0x3 == 0 {
                self.pending = Some(ch);
            }
        }
    }

    /// 执行第 `ch` 通道的完整传输（半字/字宽度、按步进搬运）。
    pub fn transfer(&mut self, ch: usize, bus: &mut crate::bus::Bus) {
        Self::run_channel(ch, &mut self.channel[ch], bus);
    }

    /// 跑完所有使能且触发码匹配的通道（VBlank=1 / HBlank=2），通道号小者优先。
    pub fn run_triggered(&mut self, timing: u16, bus: &mut crate::bus::Bus) {
        for ch in 0..4 {
            let mut c = self.channel[ch];
            if c.ctrl & 0x8000 != 0 && (c.ctrl >> 12) & 0x3 == timing {
                Self::run_channel(ch, &mut c, bus);
                self.channel[ch] = c;
            }
        }
    }

    /// 单通道搬运。通道号独立传入，供 Bus 在不借 self.io.dma 时调用。
    pub fn run_channel(ch: usize, c: &mut Channel, bus: &mut crate::bus::Bus) {
        let word = c.ctrl & 0x0400 != 0;
        let unit = if word { 4 } else { 2 };
        let dst_bits = (c.ctrl >> 5) & 0x3;
        let src_bits = (c.ctrl >> 7) & 0x3;
        let src_delta = delta(src_bits) * unit;
        let dst_delta = delta(dst_bits) * unit;

        let mut src = c.src;
        let mut dst = c.dst;
        let count: u32 = if c.count == 0 {
            match ch {
                0 => 0x4_0000,
                1 | 2 => 0x4_000,
                _ => 0x1_0000,
            }
        } else {
            u32::from(c.count)
        };
        for _ in 0..count {
            let v = if word {
                bus.read_u32(src)
            } else {
                u32::from(bus.read_u16(src))
            };
            if word {
                bus.write_u32(dst, v);
            } else {
                bus.write_u16(dst, v as u16);
            }
            src = (src as i32 + src_delta) as u32;
            dst = (dst as i32 + dst_delta) as u32;
        }
        c.src = src;
        c.dst = dst;
        let repeat = c.ctrl & 0x0200 != 0;
        let triggered = (c.ctrl >> 12) & 0x3 != 0;
        if !repeat || !triggered {
            c.ctrl &= !0x8000;
        } else if dst_bits == 3 {
            c.dst = c.dst_base;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bus::Bus;
    use crate::cartridge::Cartridge;

    fn bus() -> Bus {
        Bus::new(Cartridge::from_bytes(vec![0u8; 0x100].into()).unwrap())
    }

    #[test]
    fn immediate_transfer_copies_halfwords_and_disables() {
        let mut bus = bus();
        for i in 0..8u32 {
            bus.write_u16(0x0200_0000 + i * 2, 0x1000 + i as u16);
        }
        bus.write_u32(0x0400_00B8, 0x0200_0000);
        bus.write_u32(0x0400_00BC, 0x0200_0100);
        bus.write_u16(0x0400_00C0, 4);
        bus.write_u16(0x0400_00C2, 0x8000);
        for i in 0..4u32 {
            assert_eq!(bus.read_u16(0x0200_0100 + i * 2), 0x1000 + i as u16);
        }
        assert_eq!(bus.io.dma.channel[0].ctrl & 0x8000, 0);
        assert_eq!(bus.io.dma.channel[0].src, 0x0200_0008);
        assert_eq!(bus.io.dma.channel[0].dst, 0x0200_0108);
    }

    #[test]
    fn word_transfer_with_fixed_dst_keeps_last_value() {
        let mut bus = bus();
        for i in 0..3u32 {
            bus.write_u32(0x0200_0000 + i * 4, 0xAA00_0000 + i);
        }
        bus.write_u32(0x0400_00B8, 0x0200_0000);
        bus.write_u32(0x0400_00BC, 0x0200_0100);
        bus.write_u16(0x0400_00C0, 3);
        bus.write_u16(0x0400_00C2, 0x8440);
        assert_eq!(bus.read_u32(0x0200_0100), 0xAA00_0002);
        assert_eq!(bus.io.dma.channel[0].src, 0x0200_000C);
        assert_eq!(bus.io.dma.channel[0].dst, 0x0200_0100);
    }

    #[test]
    fn count_zero_expands_to_channel_maximum() {
        let mut bus = bus();
        bus.write_u16(0x0200_0000, 0xBEEF);
        bus.write_u32(0x0400_00C4, 0x0200_0000);
        bus.write_u32(0x0400_00C8, 0x0200_0100);
        bus.write_u16(0x0400_00CE, 0x8100);
        assert_eq!(bus.read_u16(0x0200_0100 + 0x7FFE), 0xBEEF);
        assert_eq!(bus.io.dma.channel[1].dst, 0x0200_8100);
        assert_eq!(bus.io.dma.channel[1].ctrl & 0x8000, 0);
    }

    #[test]
    fn repeat_vblank_channel_stays_enabled_for_next_frame() {
        let mut bus = bus();
        bus.write_u16(0x0200_0000, 0x1234);
        bus.write_u16(0x0200_0002, 0x5678);
        bus.write_u32(0x0400_00DC, 0x0200_0000);
        bus.write_u32(0x0400_00E0, 0x0200_0100);
        bus.write_u16(0x0400_00E4, 1);
        bus.write_u16(0x0400_00E6, 0x9200);
        assert_eq!(bus.io.dma.channel[3].ctrl & 0x8000, 0x8000);
        bus.run_dma_triggers(1);
        assert_eq!(bus.read_u16(0x0200_0100), 0x1234);
        assert_eq!(bus.io.dma.channel[3].ctrl & 0x8000, 0x8000);
        assert_eq!(bus.io.dma.channel[3].dst, 0x0200_0102);
        assert_eq!(bus.io.dma.channel[3].count, 1);
        bus.run_dma_triggers(1);
        assert_eq!(bus.read_u16(0x0200_0102), 0x5678);
    }

    #[test]
    fn reload_dst_mode_restarts_each_trigger() {
        let mut bus = bus();
        for i in 0..4u32 {
            bus.write_u16(0x0200_0000 + i * 2, 0x0300 + i as u16);
        }
        bus.write_u32(0x0400_00DC, 0x0200_0000);
        bus.write_u32(0x0400_00E0, 0x0200_0100);
        bus.write_u16(0x0400_00E4, 2);
        bus.write_u16(0x0400_00E6, 0x9260);
        bus.run_dma_triggers(1);
        bus.run_dma_triggers(1);
        assert_eq!(bus.read_u16(0x0200_0100), 0x0302);
        assert_eq!(bus.read_u16(0x0200_0102), 0x0303);
        assert_eq!(bus.io.dma.channel[3].dst, 0x0200_0100);
        assert_eq!(bus.io.dma.channel[3].src, 0x0200_0008);
    }
}
