//! 硬件窗口：决定每个像素允许哪些图层绘制，以及是否启用颜色特效。

use super::Ppu;

/// 窗口掩码位（与 GBATEK WININ/WINOUT 的图层位一一对应）。
pub const WIN_BG0: u8 = 1 << 0;
pub const WIN_BG1: u8 = 1 << 1;
pub const WIN_BG2: u8 = 1 << 2;
pub const WIN_BG3: u8 = 1 << 3;
pub const WIN_OBJ: u8 = 1 << 4;
pub const WIN_EFFECT: u8 = 1 << 5;

impl Ppu {
    /// 逐像素计算窗口掩码：覆盖所有图层与特效使能位。
    pub(crate) fn render_windows(&mut self, y: u16) {
        let win0_on = self.dispcnt & (1 << 13) != 0;
        let win1_on = self.dispcnt & (1 << 14) != 0;
        let obj_win_on = self.dispcnt & (1 << 15) != 0;

        if !win0_on && !win1_on && !obj_win_on {
            for i in 0..super::SCREEN_WIDTH {
                self.win_mask[i] = 0x3F;
            }
            return;
        }

        let win0_mask = self.winin_mask(0);
        let win1_mask = self.winin_mask(1);
        let outer_mask = self.winout_mask(true);
        let obj_mask = self.winout_mask(false);

        let w0_x = self.winh[0];
        let w1_x = self.winh[1];
        let w0_y = self.winv[0];
        let w1_y = self.winv[1];

        let in_win0 = in_range(y, w0_y) && self.dispcnt & (1 << 13) != 0;
        let in_win1 = in_range(y, w1_y) && self.dispcnt & (1 << 14) != 0;

        for x in 0..super::SCREEN_WIDTH {
            let mask = if in_win0 && in_range(x as u16, w0_x) {
                win0_mask
            } else if in_win1 && in_range(x as u16, w1_x) {
                win1_mask
            } else if obj_win_on && self.obj_window_at(x as u16, y) {
                obj_mask
            } else {
                outer_mask
            };
            self.win_mask[x] = mask;
        }
    }

    /// WININ 中某个窗口的图层/特效使能掩码。
    fn winin_mask(&self, idx: usize) -> u8 {
        // WININ 低字节 = 窗口 0，高字节 = 窗口 1
        let bits = if idx == 0 {
            self.winin & 0xFF
        } else {
            self.winin >> 8
        };
        (bits & 0x3F) as u8
    }

    /// WINOUT 外侧/OBJ 窗口的图层使能掩码。`outer` 为真取外侧字节。
    fn winout_mask(&self, outer: bool) -> u8 {
        let bits = if outer {
            self.winout & 0xFF
        } else {
            self.winout >> 8
        };
        (bits & 0x3F) as u8
    }

    /// 图层 `layer`（0-3 = BG0-3，4 = OBJ）在该像素是否允许绘制。
    pub(crate) fn win_allows(&self, x: usize, layer: usize) -> bool {
        const LAYER_BITS: [u8; 5] = [WIN_BG0, WIN_BG1, WIN_BG2, WIN_BG3, WIN_OBJ];
        self.win_mask[x] & LAYER_BITS[layer] != 0
    }

    /// 该像素是否允许颜色特效（alpha/亮度）。
    pub(crate) fn win_allows_effect(&self, x: usize) -> bool {
        self.win_mask[x] & WIN_EFFECT != 0
    }

    /// OBJ 窗口：该像素是否有标记为"OBJ 窗口模式"的精灵覆盖。
    fn obj_window_at(&self, x: u16, y: u16) -> bool {
        self.obj_window_contains(x, y)
    }
}

/// 16 位窗口坐标区间判定（含首尾，处理可能的首尾颠倒）。
fn in_range(v: u16, range: u16) -> bool {
    let lo = range >> 8;
    let hi = range & 0xFF;
    if lo <= hi {
        v >= lo && v <= hi
    } else {
        v >= lo || v <= hi
    }
}
