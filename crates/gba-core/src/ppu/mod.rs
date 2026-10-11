//! PPU：GBA 的图形处理器。

mod background;
mod bitmap;
mod blending;
mod sprite;
mod window;

use crate::frame::SCREEN_WIDTH;

pub const CYCLES_PER_LINE: u32 = 1232;
pub const VISIBLE_LINES: u16 = 160;
pub const TOTAL_LINES: u16 = 228;
pub const HDRAW_CYCLES: u32 = 1008;

pub const PALETTE_BASE: u32 = 0x0500_0000;
pub const VRAM_BASE: u32 = 0x0600_0000;
pub const OAM_BASE: u32 = 0x0700_0000;

/// 像素来源图层编号（写入 line_layer，供混合判定）。
pub const LAYER_BACKDROP: u8 = 0xFF;

pub struct Ppu {
    dispcnt: u16,
    dispstat: u16,
    vcount: u16,
    bgcnt: [u16; 4],
    bghofs: [u16; 4],
    bgvofs: [u16; 4],
    bg_affine: [[i16; 4]; 2],
    /// 仿射背景参考点（REF_X/REF_Y，BG2/BG3）。寄存器只写不读，
    /// 每条扫描线渲染时按 PA/PC 增量推进。
    bg_ref: [[i32; 2]; 2],
    winin: u16,
    winout: u16,
    winh: [u16; 2],
    winv: [u16; 2],
    bldcnt: u16,
    bldalpha: u16,
    bldy: u16,
    mosaic: u16,
    /// 当前扫描线渲染出的 240 个 BGR555 像素。
    pub(crate) line: [u16; SCREEN_WIDTH],
    pub(crate) frame555: Vec<u16>,
    /// 每像素的来源图层编号（BG0-3 = 0-3，OBJ = 4，背景色 = 0xFF）。
    pub(crate) line_layer: [u8; SCREEN_WIDTH],
    /// 每像素是否为半透明 OBJ。
    pub(crate) line_semi: [bool; SCREEN_WIDTH],
    /// 每像素的次优先图层颜色与键（alpha 混合的第二层）。
    pub(crate) line_second: [u16; SCREEN_WIDTH],
    pub(crate) line_second_layer: [u8; SCREEN_WIDTH],
    second_prio: [u8; SCREEN_WIDTH],
    /// 当前扫描线每像素的窗口掩码：位 0-3=BGlayer，位 4=OBJ，位 5=特效。
    pub(crate) win_mask: [u8; SCREEN_WIDTH],
    pub(crate) vram: Box<[u8]>,
    pub(crate) palette: Box<[u8]>,
    pub(crate) oam: Box<[u8]>,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            dispcnt: 0x0080,
            dispstat: 0,
            vcount: 0,
            bgcnt: [0; 4],
            bghofs: [0; 4],
            bgvofs: [0; 4],
            bg_affine: [[0; 4]; 2],
            bg_ref: [[0; 2]; 2],
            winin: 0,
            winout: 0,
            winh: [0; 2],
            winv: [0; 2],
            bldcnt: 0,
            bldalpha: 0,
            bldy: 0,
            mosaic: 0,
            line: [0; SCREEN_WIDTH],
            frame555: vec![0; SCREEN_WIDTH * VISIBLE_LINES as usize],
            line_layer: [LAYER_BACKDROP; SCREEN_WIDTH],
            line_semi: [false; SCREEN_WIDTH],
            line_second: [0; SCREEN_WIDTH],
            line_second_layer: [LAYER_BACKDROP; SCREEN_WIDTH],
            second_prio: [9; SCREEN_WIDTH],
            win_mask: [0xFF; SCREEN_WIDTH],
            vram: vec![0; 0x18000].into(),
            palette: vec![0; 0x400].into(),
            oam: vec![0; 0x400].into(),
        }
    }

    pub fn dispcnt(&self) -> u16 {
        self.dispcnt
    }
    pub fn dispstat(&self) -> u16 {
        self.dispstat
    }
    pub fn vcount(&self) -> u16 {
        self.vcount
    }
    pub fn bgcnt(&self, i: usize) -> u16 {
        self.bgcnt[i]
    }

    /// 由调度器在扫描线边界更新 VCOUNT。
    pub(crate) fn set_vcount(&mut self, y: u16) {
        self.vcount = y;
    }

    /// 置/清 DISPSTAT 的状态标志（位 0 = VBlank，位 1 = HBlank，位 2 = VCount）。
    pub(crate) fn set_dispstat_flag(&mut self, bit: u16, on: bool) {
        if on {
            self.dispstat |= 1 << bit;
        } else {
            self.dispstat &= !(1 << bit);
        }
    }
    pub fn winin(&self) -> u16 {
        self.winin
    }
    pub fn winout(&self) -> u16 {
        self.winout
    }
    pub fn bldcnt(&self) -> u16 {
        self.bldcnt
    }
    pub fn bldalpha(&self) -> u16 {
        self.bldalpha
    }
    pub fn bldy(&self) -> u16 {
        self.bldy
    }

    pub fn write_dispcnt(&mut self, v: u16) {
        self.dispcnt = v;
    }

    /// DISPSTAT 只允许软件写高 13 位；低 3 位是只读状态标志。
    pub fn write_dispstat(&mut self, v: u16) {
        self.dispstat = (self.dispstat & 0x0007) | (v & 0xFFF8);
    }

    pub fn write_bgcnt(&mut self, i: usize, v: u16) {
        self.bgcnt[i] = v;
    }
    pub fn write_winin(&mut self, v: u16) {
        self.winin = v;
    }
    pub fn write_winout(&mut self, v: u16) {
        self.winout = v;
    }
    pub fn write_bldcnt(&mut self, v: u16) {
        self.bldcnt = v;
    }
    pub fn write_bldalpha(&mut self, v: u16) {
        self.bldalpha = v;
    }
    pub fn write_bldy(&mut self, v: u16) {
        self.bldy = v;
    }

    /// 未在 [`crate::io`] 显式分发的 PPU 寄存器（滚动/仿射/窗口/马赛克）。
    /// `None` = 该地址不属于 PPU，由调用方决定开总线行为。
    pub fn read_reg(&self, addr: u32) -> Option<u16> {
        match addr {
            0x010..=0x016 => Some(self.bghofs[((addr - 0x010) / 2) as usize]),
            0x018..=0x01E => Some(self.bgvofs[((addr - 0x018) / 2) as usize]),
            0x020..=0x02E => {
                let reg = (((addr - 0x020) % 0x10) / 2) as usize;
                Some(self.read_affine_reg(0, reg))
            }
            0x030..=0x03E => {
                let reg = (((addr - 0x030) % 0x10) / 2) as usize;
                Some(self.read_affine_reg(1, reg))
            }
            0x040 => Some(self.winh[0]),
            0x042 => Some(self.winh[1]),
            0x044 => Some(self.winv[0]),
            0x046 => Some(self.winv[1]),
            0x048 => Some(self.mosaic),
            _ => None,
        }
    }

    fn read_affine_reg(&self, bg: usize, reg: usize) -> u16 {
        match reg {
            0..=3 => self.bg_affine[bg][reg] as u16,
            // REF 是 32 位寄存器，半字读只回显写过的部分（真机可读回）
            4..=5 => (self.bg_ref[bg][reg - 4] >> 8) as u16,
            _ => 0,
        }
    }

    pub fn write_reg(&mut self, addr: u32, v: u16) {
        match addr {
            0x010..=0x016 => self.bghofs[((addr - 0x010) / 2) as usize] = v & 0x1FF,
            0x018..=0x01E => self.bgvofs[((addr - 0x018) / 2) as usize] = v & 0x1FF,
            0x020..=0x02E => self.write_affine_reg(0, addr, v),
            0x030..=0x03E => self.write_affine_reg(1, addr, v),
            0x040 => self.winh[0] = v,
            0x042 => self.winh[1] = v,
            0x044 => self.winv[0] = v,
            0x046 => self.winv[1] = v,
            0x048 => self.mosaic = v,
            _ => {}
        }
    }

    /// BG2/BG3 仿射参数块内偏移：0-3 = PA/PB/PC/PD，4-5 = REF_X/REF_Y。
    ///
    /// REF 寄存器是 28.8 定点数，占两个半字（低半字在前）；这里只暂存
    /// 高半字，位语义由写入方按 GBATEK 的寄存器序处理。
    fn write_affine_reg(&mut self, bg: usize, addr: u32, v: u16) {
        let base = 0x020 + bg as u32 * 0x10;
        let reg = ((addr - base) % 0x10 / 2) as usize;
        match reg {
            0..=3 => self.bg_affine[bg][reg] = v as i16,
            4..=5 => self.bg_ref[bg][reg - 4] = (v as i16 as i32) << 8,
            _ => {}
        }
    }

    pub(crate) fn vram_read_u8(&self, addr: u32) -> u8 {
        let rel = ((addr - VRAM_BASE) & 0x1FFFF) as usize;
        let idx = if rel >= 0x18000 { rel - 0x8000 } else { rel };
        self.vram[idx]
    }

    pub(crate) fn vram_read_u16(&self, addr: u32) -> u16 {
        let a = (addr - VRAM_BASE) & !1;
        u16::from(self.vram_read_u8(VRAM_BASE + a))
            | (u16::from(self.vram_read_u8(VRAM_BASE + a + 1)) << 8)
    }

    pub(crate) fn palette_read_u16(&self, idx: usize) -> u16 {
        let i = (idx * 2) & 0x3FF;
        u16::from(self.palette[i]) | (u16::from(self.palette[i + 1]) << 8)
    }

    pub(crate) fn oam_read_u16(&self, offset: usize) -> u16 {
        let i = offset & 0x3FF;
        u16::from(self.oam[i]) | (u16::from(self.oam[i + 1]) << 8)
    }

    /// 渲染一条可见扫描线到 [`Ppu::line`]。
    ///
    /// 顺序：窗口掩码 → 各图层按优先级取"最优先的非透明像素" →
    /// 混合（alpha / 亮度增减）。同优先级下 OBJ 覆盖 BG。
    pub fn render_line(&mut self, y: u16) {
        let backdrop = self.backdrop_color();
        for i in 0..SCREEN_WIDTH {
            self.line[i] = backdrop;
            self.line_layer[i] = LAYER_BACKDROP;
            self.line_semi[i] = false;
            self.line_second[i] = backdrop;
            self.line_second_layer[i] = LAYER_BACKDROP;
            self.second_prio[i] = 9;
        }
        self.render_windows(y);

        let mode = self.dispcnt & 0x7;
        // 每个像素记录已选中的优先级键（越小越优先），决定谁能覆盖。
        let mut best_prio = [8u8; SCREEN_WIDTH];
        match mode {
            0 => self.render_text_bgs(y, 0b1111, &mut best_prio),
            1 => {
                self.render_text_bgs(y, 0b0011, &mut best_prio);
                self.render_affine_bg(2, y, &mut best_prio);
            }
            2 => {
                self.render_text_bgs(y, 0b0011, &mut best_prio);
                self.render_affine_bg(2, y, &mut best_prio);
                self.render_affine_bg(3, y, &mut best_prio);
            }
            3..=5 => self.render_bitmap_line(y, &mut best_prio),
            _ => {}
        }
        if self.dispcnt & (1 << 12) != 0 {
            self.render_sprites(y, &mut best_prio);
        }
        self.apply_blending();
        let y = y as usize;
        self.frame555[y * SCREEN_WIDTH..(y + 1) * SCREEN_WIDTH].copy_from_slice(&self.line);
        self.advance_affine();
    }

    /// 仿射背景的参考点按 PC/PD 逐行推进（渲染完一行后更新）。
    fn advance_affine(&mut self) {
        for bg in 0..2 {
            let pc = self.bg_affine[bg][2] as i32;
            let pd = self.bg_affine[bg][3] as i32;
            self.bg_ref[bg][0] = self.bg_ref[bg][0].wrapping_add(pc);
            self.bg_ref[bg][1] = self.bg_ref[bg][1].wrapping_add(pd);
        }
    }

    /// 候选像素竞争：只有更优先（键值更小）才允许覆盖。
    ///
    /// 键值编码 = `priority * 2 + (is_obj ? 0 : 1)`：同优先级下 OBJ 取胜
    /// （GBA 规则：OBJ 在 BG 之上；OBJ 之间按 OAM 顺序靠前者胜）。
    /// `layer` 为 4 表示 OBJ。
    pub(crate) fn try_put(
        &mut self,
        x: usize,
        layer: u8,
        priority: u8,
        color: u16,
        semi: bool,
        best_prio: &mut [u8; SCREEN_WIDTH],
    ) {
        let is_obj = layer == 4;
        let key = priority * 2 + if is_obj { 0 } else { 1 };
        if key < best_prio[x] {
            // 原获胜者降为第二层（供 alpha 混合使用）
            self.line_second[x] = self.line[x];
            self.line_second_layer[x] = self.line_layer[x];
            self.second_prio[x] = best_prio[x];
            best_prio[x] = key;
            self.line[x] = color;
            self.line_layer[x] = layer;
            self.line_semi[x] = semi;
        } else if key < self.second_prio[x] {
            self.line_second[x] = color;
            self.line_second_layer[x] = layer;
            self.second_prio[x] = key;
        }
    }

    fn backdrop_color(&self) -> u16 {
        self.palette_read_u16(0)
    }

    /// 把已留档的第 `y` 行转成 RGBA8888 写入帧缓冲。
    pub fn write_line_to_frame(&self, y: usize, frame: &mut [u8]) {
        for x in 0..SCREEN_WIDTH {
            let (r, g, b) = bgr555_to_rgb888(self.frame555[y * SCREEN_WIDTH + x]);
            let i = (y * SCREEN_WIDTH + x) * 4;
            frame[i] = r;
            frame[i + 1] = g;
            frame[i + 2] = b;
            frame[i + 3] = 0xFF;
        }
    }
}

/// BGR555 → 8 位每通道（低位 5 位左移 3 位并复制高位补足）。
pub fn bgr555_to_rgb888(c: u16) -> (u8, u8, u8) {
    let r = (c & 0x1F) as u8;
    let g = ((c >> 5) & 0x1F) as u8;
    let b = ((c >> 10) & 0x1F) as u8;
    (expand5(r), expand5(g), expand5(b))
}

fn expand5(v: u8) -> u8 {
    (v << 3) | (v >> 2)
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode4_ppu() -> Ppu {
        let mut p = Ppu::new();
        p.write_dispcnt(0x0404);
        p.palette[2] = 0xFF;
        p.palette[3] = 0x7F;
        p.palette[4] = 0x00;
        p.palette[5] = 0x7C;
        p
    }

    #[test]
    fn mode4_renders_palette_indexed_row() {
        let mut p = mode4_ppu();
        p.vram[0] = 1;
        p.vram[1] = 2;
        p.render_line(0);
        assert_eq!(p.line[0], 0x7FFF);
        assert_eq!(p.line[1], 0x7C00);
        assert_eq!(p.line[2], 0);
    }

    #[test]
    fn mode4_page_flip_selects_vram_half() {
        let mut p = mode4_ppu();
        p.vram[0xA000] = 1;
        p.render_line(0);
        assert_eq!(p.line[0], 0);
        p.write_dispcnt(0x0414);
        p.render_line(0);
        assert_eq!(p.line[0], 0x7FFF);
    }

    #[test]
    fn capture_keeps_every_rendered_line() {
        let mut p = mode4_ppu();
        p.vram[0] = 1;
        p.vram[240] = 2;
        p.render_line(0);
        p.render_line(1);
        let mut frame = vec![0u8; SCREEN_WIDTH * 160 * 4];
        p.write_line_to_frame(0, &mut frame);
        assert_eq!(&frame[0..4], &[255, 255, 255, 255]);
        p.write_line_to_frame(1, &mut frame);
        assert_eq!(&frame[960..963], &[0, 0, 255]);
    }

    #[test]
    fn disabled_windows_show_all_layers() {
        let mut p = mode4_ppu();
        p.write_winout(0x0000);
        p.vram[0] = 1;
        p.render_line(0);
        assert_eq!(p.line[0], 0x7FFF);
    }

    #[test]
    fn mode0_text_bg_maps_tiles() {
        let mut p = Ppu::new();
        p.write_dispcnt(0x0100);
        p.write_bgcnt(0, 0x0100);
        p.palette[2] = 0xFF;
        p.palette[3] = 0x7F;
        for i in 0..32usize {
            p.vram[0x800 + i * 2] = 0x01;
        }
        for b in 0..32usize {
            p.vram[32 + b] = 0x11;
        }
        p.render_line(0);
        if let Some(x) = p.line.iter().position(|&c| c != 0x7FFF) {
            eprintln!(
                "mismatch x={x} val={:#06x} entry={:#06x}",
                p.line[x],
                p.vram[0x800 + (x / 8) * 2]
            );
        }
        assert!(p.line.iter().all(|&c| c == 0x7FFF));
    }

    #[test]
    fn obj_layer_covers_bg_on_same_priority() {
        let mut p = mode4_ppu();
        p.write_dispcnt(0x1404);
        let y = 40u16;
        p.vram[y as usize * SCREEN_WIDTH + 8] = 1;
        p.oam[0] = y as u8;
        p.oam[1] = 0;
        p.oam[2] = 0;
        p.oam[3] = 0;
        p.oam[4] = 0x01;
        p.oam[5] = 0x20;
        for i in 32..64usize {
            p.vram[0x1_0000 + i] = 0x22;
        }
        p.palette[290 * 2] = 0x1F;
        p.palette[290 * 2 + 1] = 0x00;
        p.render_line(y);
        assert_eq!(p.line[0], 0x001F);
        assert_eq!(p.line[8], 0x7FFF);
    }
}
