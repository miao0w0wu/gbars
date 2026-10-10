//! I/O 寄存器层：把 0x04000000 窗口的原始字节翻译成各硬件模块的状态。

use crate::dma::Dma;
use crate::ppu::Ppu;
use crate::timer::Timers;

pub struct Io {
    pub ppu: Ppu,
    pub timers: Timers,
    pub dma: Dma,
    /// KEYINPUT（0x130）：低 10 位，0 = 按下。M3 接入真实按键。
    keyinput: u16,
    /// 中断相关寄存器的原始字节（IF/IE/IME 的真实语义随 M3 中断控制器）。
    ie: u16,
    irq_flag: u16,
    ime: u16,
    waitcnt: u16,
    /// SIO 内部时钟移位的剩余周期。FireRed 引导用 SIO 中断做随机种子：
    /// SIOCNT 的启动位写入后，8 位移位在内部时钟（256 周期/位）下完成并
    /// 置起 IF 的 SIO 位（bit7）。
    sio_shift_remaining: u32,
}

/// 一次 8 位内部时钟移位的周期数（GBATEK：内部时钟 256 周期/位）。
const SIO_SHIFT_CYCLES: u32 = 256 * 8;

impl Io {
    pub fn new() -> Self {
        Self {
            ppu: Ppu::new(),
            timers: Timers::new(),
            dma: Dma::new(),
            keyinput: 0x03FF,
            ie: 0,
            irq_flag: 0,
            ime: 0,
            waitcnt: 0,
            sio_shift_remaining: 0,
        }
    }

    /// 每步调用：SIO 移位完成时投递 SIO 中断。
    pub fn tick(&mut self, cycles: u32) {
        if self.sio_shift_remaining > 0 {
            let done = self.sio_shift_remaining <= cycles;
            self.sio_shift_remaining = self.sio_shift_remaining.saturating_sub(cycles);
            if done {
                self.request_irq(1 << 7);
            }
        }
    }

    /// 置 IF 位（中断源投递）。
    pub fn request_irq(&mut self, bits: u16) {
        self.irq_flag |= bits;
    }

    /// 清 IF 位（IntrWait 的"丢弃旧中断"语义）。
    pub fn clear_irq(&mut self, bits: u16) {
        self.irq_flag &= !bits;
    }

    /// 当前应投递的中断：IME 开 && (IE & IF) 非 0。
    pub fn pending_irq(&self) -> u16 {
        if self.ime != 0 {
            self.ie & self.irq_flag
        } else {
            0
        }
    }

    pub fn ime(&self) -> u16 {
        self.ime
    }

    pub fn read_u8(&self, addr: u32) -> u8 {
        let v = self.read_u16(addr & !1);
        if addr & 1 == 0 {
            v as u8
        } else {
            (v >> 8) as u8
        }
    }

    pub fn read_u16(&self, addr: u32) -> u16 {
        match addr & 0x3FE {
            0x000 => self.ppu.dispcnt(),
            0x004 => self.ppu.dispstat(),
            0x006 => self.ppu.vcount(),
            0x008 => self.ppu.bgcnt(0),
            0x00A => self.ppu.bgcnt(1),
            0x00C => self.ppu.bgcnt(2),
            0x00E => self.ppu.bgcnt(3),
            0x100 => self.timers.read_counter(0),
            0x104 => self.timers.read_counter(1),
            0x108 => self.timers.read_counter(2),
            0x10C => self.timers.read_counter(3),
            0x102 => self.timers.timer[0].ctrl,
            0x106 => self.timers.timer[1].ctrl,
            0x10A => self.timers.timer[2].ctrl,
            0x10E => self.timers.timer[3].ctrl,
            0x0B8 => self.dma.channel[0].src as u16,
            0x0BA => (self.dma.channel[0].src >> 16) as u16,
            0x0BC => self.dma.channel[0].dst as u16,
            0x0BE => (self.dma.channel[0].dst >> 16) as u16,
            0x0C0 => self.dma.channel[0].count,
            0x0C2 => self.dma.channel[0].ctrl,
            0x0C4 => self.dma.channel[1].src as u16,
            0x0C6 => (self.dma.channel[1].src >> 16) as u16,
            0x0C8 => self.dma.channel[1].dst as u16,
            0x0CA => (self.dma.channel[1].dst >> 16) as u16,
            0x0CC => self.dma.channel[1].count,
            0x0CE => self.dma.channel[1].ctrl,
            0x0D0 => self.dma.channel[2].src as u16,
            0x0D2 => (self.dma.channel[2].src >> 16) as u16,
            0x0D4 => self.dma.channel[2].dst as u16,
            0x0D6 => (self.dma.channel[2].dst >> 16) as u16,
            0x0D8 => self.dma.channel[2].count,
            0x0DA => self.dma.channel[2].ctrl,
            0x0DC => self.dma.channel[3].src as u16,
            0x0DE => (self.dma.channel[3].src >> 16) as u16,
            0x0E0 => self.dma.channel[3].dst as u16,
            0x0E2 => (self.dma.channel[3].dst >> 16) as u16,
            0x0E4 => self.dma.channel[3].count,
            0x0E6 => self.dma.channel[3].ctrl,
            0x048 => self.ppu.winin(),
            0x04A => self.ppu.winout(),
            0x050 => self.ppu.bldcnt(),
            0x052 => self.ppu.bldalpha(),
            0x054 => self.ppu.bldy(),
            0x130 => self.keyinput,
            0x132 => 0,
            0x134 => 0,
            0x200 => self.ie,
            0x202 => self.irq_flag,
            0x204 => self.waitcnt,
            0x208 => self.ime,
            _ => self.ppu.read_reg(addr & 0x3FE),
        }
    }

    pub fn write_u8(&mut self, addr: u32, value: u8) -> Option<usize> {
        let cur = self.read_u16(addr & !1);
        let merged = if addr & 1 == 0 {
            (cur & 0xFF00) | u16::from(value)
        } else {
            (cur & 0x00FF) | (u16::from(value) << 8)
        };
        self.write_u16(addr & !1, merged)
    }

    /// 写 DMA 控制字；使能位 0→1 且触发码为"立即"时返回通道号。
    fn write_dma_ctrl(&mut self, ch: usize, value: u16) -> Option<usize> {
        let was = self.dma.enabled(ch);
        self.dma.write_ctrl(ch, value);
        (!was && self.dma.enabled(ch) && (value >> 12) & 0x3 == 0).then_some(ch)
    }

    /// 返回 Some(ch) 表示该写触发了一次立即 DMA 传输（由 Bus 执行搬运）。
    pub fn write_u16(&mut self, addr: u32, value: u16) -> Option<usize> {
        let trigger: Option<usize> = match addr & 0x3FE {
            0x000 => { self.ppu.write_dispcnt(value); None }
            0x004 => { self.ppu.write_dispstat(value); None }
            0x008 => { self.ppu.write_bgcnt(0, value); None }
            0x00A => { self.ppu.write_bgcnt(1, value); None }
            0x00C => { self.ppu.write_bgcnt(2, value); None }
            0x00E => { self.ppu.write_bgcnt(3, value); None }
            0x100 => { self.timers.write_reload(0, value); None }
            0x104 => { self.timers.write_reload(1, value); None }
            0x108 => { self.timers.write_reload(2, value); None }
            0x10C => { self.timers.write_reload(3, value); None }
            0x102 => { self.timers.write_ctrl(0, value); None }
            0x106 => { self.timers.write_ctrl(1, value); None }
            0x10A => { self.timers.write_ctrl(2, value); None }
            0x10E => { self.timers.write_ctrl(3, value); None }
            0x0B8 => {
                self.dma.channel[0].src = (self.dma.channel[0].src & 0xFFFF_0000) | u32::from(value);
                None
            }
            0x0BA => {
                self.dma.channel[0].src =
                    (self.dma.channel[0].src & 0x0000_FFFF) | (u32::from(value) << 16);
                None
            }
            0x0BC => {
                self.dma.channel[0].dst = (self.dma.channel[0].dst & 0xFFFF_0000) | u32::from(value);
                None
            }
            0x0BE => {
                self.dma.channel[0].dst =
                    (self.dma.channel[0].dst & 0x0000_FFFF) | (u32::from(value) << 16);
                None
            }
            0x0C0 => {
                self.dma.channel[0].count = value;
                None
            }
            0x0C2 => self.write_dma_ctrl(0, value),
            0x0C4 => {
                self.dma.channel[1].src = (self.dma.channel[1].src & 0xFFFF_0000) | u32::from(value);
                None
            }
            0x0C6 => {
                self.dma.channel[1].src =
                    (self.dma.channel[1].src & 0x0000_FFFF) | (u32::from(value) << 16);
                None
            }
            0x0C8 => {
                self.dma.channel[1].dst = (self.dma.channel[1].dst & 0xFFFF_0000) | u32::from(value);
                None
            }
            0x0CA => {
                self.dma.channel[1].dst =
                    (self.dma.channel[1].dst & 0x0000_FFFF) | (u32::from(value) << 16);
                None
            }
            0x0CC => {
                self.dma.channel[1].count = value;
                None
            }
            0x0CE => self.write_dma_ctrl(1, value),
            0x0D0 => {
                self.dma.channel[2].src = (self.dma.channel[2].src & 0xFFFF_0000) | u32::from(value);
                None
            }
            0x0D2 => {
                self.dma.channel[2].src =
                    (self.dma.channel[2].src & 0x0000_FFFF) | (u32::from(value) << 16);
                None
            }
            0x0D4 => {
                self.dma.channel[2].dst = (self.dma.channel[2].dst & 0xFFFF_0000) | u32::from(value);
                None
            }
            0x0D6 => {
                self.dma.channel[2].dst =
                    (self.dma.channel[2].dst & 0x0000_FFFF) | (u32::from(value) << 16);
                None
            }
            0x0D8 => {
                self.dma.channel[2].count = value;
                None
            }
            0x0DA => self.write_dma_ctrl(2, value),
            0x0DC => {
                self.dma.channel[3].src = (self.dma.channel[3].src & 0xFFFF_0000) | u32::from(value);
                None
            }
            0x0DE => {
                self.dma.channel[3].src =
                    (self.dma.channel[3].src & 0x0000_FFFF) | (u32::from(value) << 16);
                None
            }
            0x0E0 => {
                self.dma.channel[3].dst = (self.dma.channel[3].dst & 0xFFFF_0000) | u32::from(value);
                None
            }
            0x0E2 => {
                self.dma.channel[3].dst =
                    (self.dma.channel[3].dst & 0x0000_FFFF) | (u32::from(value) << 16);
                None
            }
            0x0E4 => {
                self.dma.channel[3].count = value;
                None
            }
            0x0E6 => self.write_dma_ctrl(3, value),
            0x048 => { self.ppu.write_winin(value); None }
            0x04A => { self.ppu.write_winout(value); None }
            0x050 => { self.ppu.write_bldcnt(value); None }
            0x052 => { self.ppu.write_bldalpha(value); None }
            0x054 => { self.ppu.write_bldy(value); None }
            0x130 => None,
            0x134 => {
                // SIOCNT：bit7 = 启动移位（内部时钟模式）
                if value & 0x0080 != 0 {
                    self.sio_shift_remaining = SIO_SHIFT_CYCLES;
                }
                None
            }
            0x200 => {
                #[cfg(feature = "trace-io")]
                eprintln!("IE = {value:#06x} (at write_u16)");
                self.ie = value;
                None
            }
            0x202 => {
                self.irq_flag &= !value;
                None
            }
            0x204 => {
                self.waitcnt = value;
                None
            }
            0x208 => {
                self.ime = value & 1;
                None
            }
            _ => {
                #[cfg(feature = "trace-io")]
                eprintln!("IO write {:#06x} = {:#06x}", addr & 0x3FE, value);
                self.ppu.write_reg(addr & 0x3FE, value);
                None
            }
        };
        trigger
    }
}

impl Default for Io {
    fn default() -> Self {
        Self::new()
    }
}