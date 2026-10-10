//! OAM 精灵渲染（普通/仿射/翻转/半透明/OBJ 窗口）。

use super::{Ppu, VRAM_BASE};

/// 各形状（方形/横长/竖长）× 尺寸编码的像素边长 (宽, 高)。
const SPRITE_SIZES: [[(i32, i32); 4]; 3] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)], // 方形
    [(16, 8), (32, 8), (32, 16), (64, 32)], // 横长
    [(8, 16), (8, 32), (16, 32), (32, 64)], // 竖长
];

impl Ppu {
    /// 渲染所有 OBJ。OAM 索引靠前者在优先级相同的情形下绘制在上。
    pub(crate) fn render_sprites(&mut self, y: u16, best_prio: &mut [u8; super::SCREEN_WIDTH]) {
        let one_dim = self.dispcnt & (1 << 6) != 0;
        let bitmap_mode = self.dispcnt & (1 << 7) != 0;
        for index in (0..128).rev() {
            let attr0 = self.oam_read_u16(index * 8);
            let attr1 = self.oam_read_u16(index * 8 + 2);
            let attr2 = self.oam_read_u16(index * 8 + 4);
            self.render_one_sprite(
                index,
                attr0,
                attr1,
                attr2,
                y,
                one_dim,
                bitmap_mode,
                best_prio,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_one_sprite(
        &mut self,
        _index: usize,
        attr0: u16,
        attr1: u16,
        attr2: u16,
        y: u16,
        one_dim: bool,
        bitmap_mode: bool,
        best_prio: &mut [u8; super::SCREEN_WIDTH],
    ) {
        let shape = ((attr0 >> 14) & 0x3) as usize;
        if shape > 2 {
            return;
        }
        let size_code = ((attr1 >> 14) & 0x3) as usize;
        let (w, h) = SPRITE_SIZES[shape][size_code];

        let affine = attr0 & (1 << 8) != 0;
        let rot_scale = attr0 & (1 << 9) != 0;
        // 非仿射精灵的位 9 是"禁用"标志（置 1 则整个精灵不显示）
        if !affine && rot_scale {
            return;
        }
        let double_size = affine && rot_scale;
        // 位 10-11 = OBJ 模式：0 = 普通，1 = 半透明，2 = OBJ 窗口，3 保留
        let obj_mode = (attr0 >> 10) & 0x3;
        let obj_window = obj_mode == 2;
        let semi = obj_mode == 1;
        let mosaic = attr0 & (1 << 12) != 0;
        let color_8bpp = attr0 & (1 << 13) != 0;

        let priority = ((attr2 >> 10) & 0x3) as u8;
        let tile = (attr2 & 0x3FF) as u32;
        let pal_bank = ((attr2 >> 12) & 0xF) as usize;

        // 位置：8 位有符号（相对屏幕 128 偏移），这里按 GBATEK 直接取低 8/9 位
        let sp_y = (attr0 & 0xFF) as i32;
        let sp_x = (attr1 & 0x1FF) as i32;

        let (draw_w, draw_h) = if double_size { (w * 2, h * 2) } else { (w, h) };
        let ly = y as i32 - sp_y;
        if ly < 0 || ly >= draw_h {
            return;
        }

        let (pa, pb, pc, pd) = if affine {
            let base = 0x06 + ((attr1 >> 9) & 0x1F) as usize * 8;
            let pa = self.oam_read_u16(base) as i16 as i32;
            let pb = self.oam_read_u16(base + 2) as i16 as i32;
            let pc = self.oam_read_u16(base + 4) as i16 as i32;
            let pd = self.oam_read_u16(base + 6) as i16 as i32;
            (pa, pb, pc, pd)
        } else {
            (0x100, 0, 0, 0x100)
        };

        let mosaic_h = if mosaic && self.mosaic & (1 << 4) != 0 {
            ((self.mosaic & 0xF) as i32) + 1
        } else {
            1
        };
        let mosaic_v = if mosaic && self.mosaic & (1 << 4) != 0 {
            (((self.mosaic >> 4) & 0xF) as i32) + 1
        } else {
            1
        };

        let hflip = !affine && attr1 & (1 << 12) != 0;
        let vflip = !affine && attr1 & (1 << 13) != 0;
        let half_w = w / 2;
        let half_h = h / 2;

        for sx in 0..super::SCREEN_WIDTH {
            let lx = sx as i32 - sp_x;
            if lx < 0 || lx >= draw_w {
                continue;
            }

            let (tx, ty) = if affine {
                let cx = lx - draw_w / 2;
                let cy = ly - draw_h / 2;
                let px = (pa * cx + pb * cy) >> 8;
                let py = (pc * cx + pd * cy) >> 8;
                (px + half_w, py + half_h)
            } else {
                let fx = if hflip { w - 1 - lx } else { lx };
                let fy = if vflip { h - 1 - ly } else { ly };
                (fx, fy)
            };
            if tx < 0 || ty < 0 || tx >= w || ty >= h {
                continue;
            }
            let mut tx = tx as u32;
            let mut ty = ty as u32;
            if mosaic_h > 1 {
                tx &= !(mosaic_h as u32 - 1);
            }
            if mosaic_v > 1 {
                ty &= !(mosaic_v as u32 - 1);
            }

            let color_idx = if bitmap_mode {
                self.sprite_bitmap_pixel(tile, tx, ty)
            } else {
                self.sprite_tile_pixel(tile, w, tx, ty, color_8bpp, one_dim)
            };
            if color_idx == 0 {
                continue;
            }
            if obj_window || self.win_mask[sx] & super::window::WIN_OBJ == 0 {
                continue;
            }

            let pal_index = if color_8bpp {
                color_idx + 256
            } else {
                256 + pal_bank * 16 + color_idx
            };
            let color = self.palette_read_u16(pal_index);
            self.try_put(sx, 4, priority, color, semi, best_prio);
        }
    }

    /// 图块精灵取色（4bpp/8bpp，1D/2D 映射）。
    fn sprite_tile_pixel(
        &self,
        tile: u32,
        w: i32,
        tx: u32,
        ty: u32,
        color_8bpp: bool,
        one_dim: bool,
    ) -> usize {
        let tile_x = tx / 8;
        let tile_y = ty / 8;
        let tile_w = (w as u32) / 8;
        let tile_index = if one_dim {
            tile + tile_y * tile_w + tile_x
        } else {
            let stride = if color_8bpp { 16 } else { 32 };
            tile + tile_y * stride + tile_x
        };
        let px = tx & 7;
        let py = ty & 7;
        if color_8bpp {
            let addr = VRAM_BASE + 0x10000 + tile_index * 64 + py * 8 + px;
            self.vram_read_u8(addr) as usize
        } else {
            let addr = VRAM_BASE + 0x10000 + tile_index * 32 + py * 4 + px / 2;
            let byte = self.vram_read_u8(addr) as usize;
            if px & 1 == 0 { byte & 0xF } else { byte >> 4 }
        }
    }

    /// 位图模式精灵取色（256 色，数据在 OBJ 区 0x10000）。
    fn sprite_bitmap_pixel(&self, tile: u32, tx: u32, ty: u32) -> usize {
        // 位图精灵：tile 为半字索引，行宽 240 半字（= 480 字节）
        let byte_off = tile * 32 + ty * 240 * 2 + tx;
        self.vram_read_u8(VRAM_BASE + 0x10000 + byte_off) as usize
    }

    /// 该像素是否被某个 OBJ 窗口精灵覆盖（供 window.rs）。
    pub(crate) fn obj_window_contains(&self, x: u16, y: u16) -> bool {
        for index in 0..128 {
            let attr0 = self.oam_read_u16(index * 8);
            if (attr0 >> 10) & 0x3 != 2 || attr0 & (1 << 8) != 0 {
                continue; // 只有普通（非仿射）OBJ 窗口精灵参与
            }
            let shape = ((attr0 >> 14) & 0x3) as usize;
            if shape > 2 {
                continue;
            }
            let attr1 = self.oam_read_u16(index * 8 + 2);
            let (w, h) = SPRITE_SIZES[shape][((attr1 >> 14) & 0x3) as usize];
            let sp_y = (attr0 & 0xFF) as i32;
            let sp_x = (attr1 & 0x1FF) as i32;
            let lx = x as i32 - sp_x;
            let ly = y as i32 - sp_y;
            if lx >= 0 && lx < w && ly >= 0 && ly < h {
                return true;
            }
        }
        false
    }
}
