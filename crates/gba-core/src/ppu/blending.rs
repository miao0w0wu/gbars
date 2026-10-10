//! 颜色特效：alpha 混合与亮度增减（BLDCNT/BLDALPHA/BLDY）。

use super::{LAYER_BACKDROP, Ppu};

const BLD_OBJ: u16 = 1 << 4;
const BLD_BACKDROP: u16 = 1 << 5;

impl Ppu {
    pub(crate) fn apply_blending(&mut self) {
        let effect = (self.bldcnt >> 6) & 0x3;
        if effect == 0 {
            return;
        }
        let first_targets = self.bldcnt & 0x3F;
        let second_targets = (self.bldcnt >> 8) & 0x3F;
        let eva = (self.bldalpha & 0x1F).min(16) as i32;
        let evb = ((self.bldalpha >> 8) & 0x1F).min(16) as i32;
        let evy = (self.bldy & 0x1F).min(16) as i32;

        for x in 0..super::SCREEN_WIDTH {
            if !self.win_allows_effect(x) {
                continue;
            }
            let top_layer = self.line_layer[x];
            let top_is_target = layer_targeted(top_layer, first_targets);
            let force_semi = top_layer == 4 && self.line_semi[x];
            if !top_is_target && !force_semi {
                continue;
            }
            match effect {
                1 => {
                    // alpha 混合：第一层（上）与第二层（下）按 EVA/EVB 混合。
                    // 只有第二层也参与时才有效；半透明 OBJ 强制参与第一层。
                    let second = self.line_second[x];
                    let use_second =
                        force_semi || layer_targeted(self.line_second_layer[x], second_targets);
                    if use_second {
                        let bottom = if self.line_second_layer[x] == LAYER_BACKDROP {
                            self.palette_read_u16(0)
                        } else {
                            second
                        };
                        self.line[x] = alpha_blend(self.line[x], bottom, eva, evb);
                    }
                }
                2 => self.line[x] = brightness_up(self.line[x], evy),
                3 => self.line[x] = brightness_down(self.line[x], evy),
                _ => {}
            }
        }
    }
}

fn layer_targeted(layer: u8, targets: u16) -> bool {
    if layer == LAYER_BACKDROP {
        targets & BLD_BACKDROP != 0
    } else if layer == 4 {
        targets & BLD_OBJ != 0
    } else {
        targets & (1 << layer) != 0
    }
}

fn alpha_blend(top: u16, bottom: u16, eva: i32, evb: i32) -> u16 {
    let mut out = 0u16;
    for shift in [0u32, 5, 10] {
        let a = ((top >> shift) & 0x1F) as i32;
        let b = ((bottom >> shift) & 0x1F) as i32;
        let v = ((a * eva + b * evb) >> 4).clamp(0, 31) as u16;
        out |= v << shift;
    }
    out
}

fn brightness_up(c: u16, evy: i32) -> u16 {
    let mut out = 0u16;
    for shift in [0u32, 5, 10] {
        let v = ((c >> shift) & 0x1F) as i32;
        let v = v + (((31 - v) * evy) >> 4);
        out |= (v.clamp(0, 31) as u16) << shift;
    }
    out
}

fn brightness_down(c: u16, evy: i32) -> u16 {
    let mut out = 0u16;
    for shift in [0u32, 5, 10] {
        let v = ((c >> shift) & 0x1F) as i32;
        let v = v - ((v * evy) >> 4);
        out |= (v.clamp(0, 31) as u16) << shift;
    }
    out
}
