//! DMA：4 通道拷贝引擎（立即触发 + VBlank/HBlank/Special 触发）。

/// 单通道可编程状态（寄存器镜像）。
#[derive(Clone, Copy, Default)]
pub struct Channel {
    pub src: u32,
    pub dst: u32,
    pub count: u16,
    /// DMAxCNT_H（0x040000B8+0xA）：
    /// 位 5-6 目标步进，位 7-8 源步进，位 10-11 触发，位 12 重复，
    /// 位 13 半字/字，位 14 中断使能，位 15 使能。
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

    /// 写控制字。使能位的 0→1 边沿 + 触发码 0（立即）时暂存通道号，
    /// 由 Bus 的写路径统一执行。
    pub fn write_ctrl(&mut self, ch: usize, v: u16) {
        let was = self.enabled(ch);
        self.channel[ch].ctrl = v;
        if !was && self.enabled(ch) && (v >> 12) & 0x3 == 0 {
            self.pending = Some(ch);
        }
    }

    /// 执行第 `ch` 通道的完整传输（半字/字宽度、按步进搬运）。
    pub fn transfer(&mut self, ch: usize, bus: &mut crate::bus::Bus) {
        Self::run_channel(ch, &mut self.channel[ch], bus);
    }

    /// 单通道搬运。通道号独立传入，供 Bus 在不借 self.io.dma 时调用。
    pub fn run_channel(ch: usize, c: &mut Channel, bus: &mut crate::bus::Bus) {
        let word = c.ctrl & 0x0400 != 0;
        let unit = if word { 4 } else { 2 };
        let dst_bits = (c.ctrl >> 5) & 0x3;
        let src_bits = (c.ctrl >> 7) & 0x3;
        let src_delta = delta(src_bits) * unit as i32 / 2;
        let dst_delta = delta(dst_bits) * unit as i32 / 2;

        let mut src = c.src;
        let mut dst = c.dst;
        let count: u32 = if c.count == 0 {
            match ch {
                0 => 0x4_0000,
                1 | 2 => 0x400,
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
        // 非重复模式：传输完自动关闭
        if c.ctrl & 0x0200 == 0 {
            c.ctrl &= !0x8000;
        }
    }
}