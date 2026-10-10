//! 位图模式渲染（模式 3/4/5）。

use super::{Ppu, VRAM_BASE};

impl Ppu {
    /// 位图模式每条扫描线只有一个背景层（BG2），优先级固定为 0。
    pub(crate) fn render_bitmap_line(&mut self, y: u16, best_prio: &mut [u8; super::SCREEN_WIDTH]) {
        if self.dispcnt & (1 << 10) == 0 {
            return; // 位图模式也要 BG2 使能位
        }
        match self.dispcnt & 0x7 {
            3 => self.render_mode3(y, best_prio),
            4 => self.render_mode4(y, best_prio),
            5 => self.render_mode5(y, best_prio),
            _ => {}
        }
    }

    /// 模式 3：240×160 直接 16bpp，帧缓冲从 VRAM 起始处开始。
    fn render_mode3(&mut self, y: u16, best_prio: &mut [u8; super::SCREEN_WIDTH]) {
        let row = VRAM_BASE + (y as u32) * 240 * 2;
        for x in 0..super::SCREEN_WIDTH {
            if !self.win_allows(x, 2) {
                continue;
            }
            let color = self.vram_read_u16(row + (x as u32) * 2);
            self.try_put(x, 2, 0, color, false, best_prio);
        }
    }

    /// 模式 4：240×160 8bpp 索引色，双帧（帧选择 = DISPCNT 位 4）。
    fn render_mode4(&mut self, y: u16, best_prio: &mut [u8; super::SCREEN_WIDTH]) {
        let frame_off = if self.dispcnt & (1 << 4) != 0 {
            0xA000
        } else {
            0
        };
        let row = VRAM_BASE + frame_off + (y as u32) * 240;
        for x in 0..super::SCREEN_WIDTH {
            if !self.win_allows(x, 2) {
                continue;
            }
            let idx = self.vram_read_u8(row + x as u32) as usize;
            if idx == 0 {
                continue;
            }
            let color = self.palette_read_u16(idx);
            self.try_put(x, 2, 0, color, false, best_prio);
        }
    }

    /// 模式 5：160×128 16bpp 双帧，超出部分环绕。
    fn render_mode5(&mut self, y: u16, best_prio: &mut [u8; super::SCREEN_WIDTH]) {
        let frame_off = if self.dispcnt & (1 << 4) != 0 {
            0xA000
        } else {
            0
        };
        let yy = (y % 128) as u32;
        let row = VRAM_BASE + frame_off + yy * 160 * 2;
        for x in 0..super::SCREEN_WIDTH {
            if !self.win_allows(x, 2) {
                continue;
            }
            let xx = (x % 160) as u32;
            let color = self.vram_read_u16(row + xx * 2);
            self.try_put(x, 2, 0, color, false, best_prio);
        }
    }
}
