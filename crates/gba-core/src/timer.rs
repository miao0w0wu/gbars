//! 定时器：4×16 位，分频 1/64/256/1024，级联模式。

#[derive(Clone, Copy, Default)]
pub struct Timer {
    /// TMxCNT_L 写入的重载值。
    pub reload: u16,
    /// 当前计数值（读取 TMxCNT_L 的返回）。
    pub counter: u16,
    /// TMxCNT_H：位 6=中断使能，位 7=启用，位 2=级联，位 1-0=分频。
    pub ctrl: u16,
    /// 分频相位累计（不足一次 tick 的周期余数）。
    subcount: u32,
}

impl Timer {
    pub fn enabled(&self) -> bool {
        self.ctrl & 0x80 != 0
    }

    pub fn cascade(&self) -> bool {
        self.ctrl & 0x4 != 0
    }

    pub fn irq_enabled(&self) -> bool {
        self.ctrl & 0x40 != 0
    }

    pub fn prescale(&self) -> u32 {
        match self.ctrl & 0x3 {
            0 => 1,
            1 => 64,
            2 => 256,
            _ => 1024,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Timers {
    pub timer: [Timer; 4],
}

impl Timers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_counter(&self, idx: usize) -> u16 {
        self.timer[idx].counter
    }

    /// 写 TMxCNT_L：真机直接改写当前计数值。
    pub fn write_reload(&mut self, idx: usize, v: u16) {
        self.timer[idx].reload = v;
        self.timer[idx].counter = v;
    }

    /// 写 TMxCNT_H。位 7 的 0→1 边沿把 counter 置为 reload 并清分频相位。
    pub fn write_ctrl(&mut self, idx: usize, v: u16) {
        let t = &mut self.timer[idx];
        let was = t.enabled();
        t.ctrl = v & 0xC7;
        if !was && t.enabled() {
            t.counter = t.reload;
            t.subcount = 0;
        }
    }

    /// 推进 `cycles` 周期，返回溢出的定时器位掩码（bit0-3）。
    /// 级联定时器由上级溢出驱动，不消耗自身分频节拍。
    pub fn tick(&mut self, cycles: u32) -> u8 {
        let mut overflowed = 0u8;
        for i in 0..4 {
            let t = self.timer[i];
            if !t.enabled() || t.cascade() {
                continue;
            }
            let ps = t.prescale();
            let total = t.subcount + cycles;
            let ticks = total / ps;
            self.timer[i].subcount = total % ps;
            overflowed |= self.tick_counter(i, ticks);
        }
        overflowed
    }

    /// 给第 `idx` 个定时器加 `ticks` 个计数节拍（含级联传播）。
    fn tick_counter(&mut self, idx: usize, mut ticks: u32) -> u8 {
        let mut overflowed = 0u8;
        while ticks > 0 {
            let t = self.timer[idx];
            let remain = 0x1_0000 - u32::from(t.counter);
            if ticks < remain {
                self.timer[idx].counter += ticks as u16;
                break;
            }
            ticks -= remain;
            self.timer[idx].counter = t.reload;
            overflowed |= 1 << idx;
            if idx + 1 < 4 {
                overflowed |= self.tick_cascade(idx + 1);
            }
        }
        overflowed
    }

    /// 级联驱动：下一级 +1，可能连锁溢出。
    fn tick_cascade(&mut self, idx: usize) -> u8 {
        let t = self.timer[idx];
        let (c, of) = t.counter.overflowing_add(1);
        self.timer[idx].counter = if of { t.reload } else { c };
        if of {
            let mut mask = 1 << idx;
            if idx + 1 < 4 {
                mask |= self.tick_cascade(idx + 1);
            }
            mask
        } else {
            0
        }
    }
}