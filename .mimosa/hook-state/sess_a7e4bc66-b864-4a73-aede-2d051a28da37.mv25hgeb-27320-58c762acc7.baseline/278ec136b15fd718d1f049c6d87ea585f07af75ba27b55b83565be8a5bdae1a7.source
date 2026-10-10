//! 背景图层渲染：文本模式（BG0–BG3）与仿射模式（BG2/BG3）。

use super::{Ppu, VRAM_BASE};

impl Ppu {
    /// 文本模式背景。`enabled_mask` 指定本显示模式下哪些 BG 走文本路径。
    pub(crate) fn render_text_bgs(
        &mut self,
        y: u16,
        enabled_mask: u8,
        best_prio: &mut [u8; super::SCREEN_WIDTH],
    ) {
        for bg in 0..4usize {
            if enabled_mask & (1 << bg) == 0 {
                continue;
            }
            if self.dispcnt & (1 << (8 + bg)) == 0 {
                continue;
            }
            self.render_text_bg(bg, y, best_prio);
        }
    }

    fn render_text_bg(&mut self, bg: usize, y: u16, best_prio: &mut [u8; super::SCREEN_WIDTH]) {
        let cnt = self.bgcnt[bg];
        let char_base = ((cnt & 0x000F) as u32) * 0x4000;
        // BG0 只有 4bpp；BG1–BG3 的位 7 选择 8bpp。
        let color_8bpp = bg != 0 && cnt & 0x0080 != 0;
        let screen_base = (((cnt & 0x1F00) >> 8) as u32) * 0x800;
        let (size_w, size_h) = match (cnt >> 14) & 0x3 {
            0 => (256u32, 256u32),
            1 => (512, 256),
            2 => (256, 512),
            _ => (512, 512),
        };
        let hofs = self.bghofs[bg] as u32;
        let vofs = self.bgvofs[bg] as u32;
        let yy = (y as u32 + vofs) % size_h;

        let mosaic_enabled = self.mosaic & (1 << bg) != 0;
        let mosaic_h = if mosaic_enabled {
            ((self.mosaic & 0xF) as u32) + 1
        } else {
            1
        };
        let mosaic_v = if mosaic_enabled {
            (((self.mosaic >> 4) & 0xF) as u32) + 1
        } else {
            1
        };
        let yv = yy & !(mosaic_v - 1);
        let priority = (cnt & 0x3) as u8;

        for x in 0..super::SCREEN_WIDTH {
            if !self.win_allows(x, bg) {
                continue;
            }
            let xx = (x as u32 + hofs) % size_w;
            let xv = xx & !(mosaic_h - 1);

            let tile_x = xv / 8;
            let tile_y = yv / 8;
            // 文本地图按 32×32 图块分块：512 尺寸时右/下半区各偏移 0x800，
            // 512×512 的右下角两块叠加偏移到 0x1000（GBATEK "BG Map"）。
            let half_x = size_w == 512 && tile_x >= 32;
            let half_y = size_h == 512 && tile_y >= 32;
            let map_offset = screen_base
                + (tile_y % 32) * 32
                + (tile_x % 32)
                + if half_x { 0x800 } else { 0 }
                + if half_y { 0x800 } else { 0 };
            let entry = self.vram_read_u16(VRAM_BASE + map_offset * 2);
            let tile_index = (entry & 0x3FF) as u32;
            let hflip = entry & 0x400 != 0;
            let vflip = entry & 0x800 != 0;
            let pal_bank = ((entry >> 12) & 0xF) as usize;

            let px = xv & 7;
            let py = yv & 7;
            let fx = if hflip { 7 - px } else { px };
            let fy = if vflip { 7 - py } else { py };

            let (color_idx, pal_index) = if color_8bpp {
                let tile_addr = char_base + tile_index * 64 + fy * 8 + fx;
                let c = self.vram_read_u8(VRAM_BASE + tile_addr) as usize;
                (c, c)
            } else {
                let tile_addr = char_base + tile_index * 32 + fy * 4 + fx / 2;
                let byte = self.vram_read_u8(VRAM_BASE + tile_addr) as usize;
                let nib = if fx & 1 == 0 { byte & 0xF } else { byte >> 4 };
                (nib, pal_bank * 16 + nib)
            };
            if color_idx == 0 {
                continue;
            }
            let color = self.palette_read_u16(pal_index);
            self.try_put(x, bg as u8, priority, color, false, best_prio);
        }
    }

    /// 仿射背景（BG2/BG3）。参考点用 PA/PC 横向、PB/PD 纵向计算纹理坐标。
    pub(crate) fn render_affine_bg(
        &mut self,
        bg: usize,
        y: u16,
        best_prio: &mut [u8; super::SCREEN_WIDTH],
    ) {
        let cnt = self.bgcnt[bg];
        let char_base = ((cnt & 0x000F) as u32) * 0x4000;
        let screen_base = (((cnt & 0x1F00) >> 8) as u32) * 0x800;
        let wrap = cnt & 0x2000 != 0;
        let size = ((cnt >> 14) & 0x3) as u32;
        let (map_w, map_h) = match size {
            0 => (128i32, 128i32),
            1 => (256, 256),
            2 => (512, 512),
            _ => (1024, 1024),
        };
        let priority = (cnt & 0x3) as u8;
        let tiles_per_row = map_w as u32 / 8;

        let aff = self.bg_affine[bg - 2];
        let pa = aff[0] as i32;
        let pb = aff[1] as i32;
        let pc = aff[2] as i32;
        let pd = aff[3] as i32;
        let ref_x = self.bg_ref[bg - 2][0];
        let ref_y = self.bg_ref[bg - 2][1];

        let mosaic_enabled = self.mosaic & (1 << bg) != 0;
        let mosaic_h = if mosaic_enabled {
            ((self.mosaic & 0xF) as i32) + 1
        } else {
            1
        };
        let mosaic_v = if mosaic_enabled {
            (((self.mosaic >> 4) & 0xF) as i32) + 1
        } else {
            1
        };
        let vy = (y as i32) & !(mosaic_v - 1);

        for x in 0..super::SCREEN_WIDTH {
            if !self.win_allows(x, bg) {
                continue;
            }
            let vx = (x as i32) & !(mosaic_h - 1);
            let mut tex_x = (ref_x + pa * vx + pb * vy) >> 8;
            let mut tex_y = (ref_y + pc * vx + pd * vy) >> 8;
            if wrap {
                tex_x = tex_x.rem_euclid(map_w);
                tex_y = tex_y.rem_euclid(map_h);
            } else if tex_x < 0 || tex_y < 0 || tex_x >= map_w || tex_y >= map_h {
                continue;
            }
            let map_offset =
                screen_base + (tex_y as u32 >> 3) * tiles_per_row + (tex_x as u32 >> 3);
            let tile_index = self.vram_read_u8(VRAM_BASE + map_offset) as u32;
            let tile_addr =
                char_base + tile_index * 64 + ((tex_y as u32) & 7) * 8 + ((tex_x as u32) & 7);
            let color_idx = self.vram_read_u8(VRAM_BASE + tile_addr);
            if color_idx == 0 {
                continue;
            }
            let color = self.palette_read_u16(color_idx as usize);
            self.try_put(x, bg as u8, priority, color, false, best_prio);
        }
    }
}
