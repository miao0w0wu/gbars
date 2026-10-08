//! 帧输出契约：模拟核心 → 前端的视频接口。
//!
//! # 为什么需要"契约"
//!
//! 核心与前端是两个世界（Rust 模拟线程 / WebView 里的 React），中间隔着
//! Tauri IPC。两边必须对"一帧视频长什么样"达成完全一致的约定——本模块
//! 就是这个约定的代码化：
//!
//! - **尺寸**：240 × 160（GBA 真实分辨率，宽高比 3:2）；
//! - **像素格式**：RGBA8888——每像素 4 字节，按 R、G、B、A 顺序，A 恒为
//!   0xFF。选它有私心：这是浏览器 canvas `ImageData` 的原生格式，前端
//!   拿到字节就能直接上屏，零转换；
//! - **存储顺序**：行主序——最前面的 960 字节是第 0 行（屏幕最上面），
//!   依次往下。一帧共 240×160×4 = 153,600 字节。
//!
//! # M0 的测试帧
//!
//! PPU（M2）实现之前，核心产不出真实画面。为了让"核心产帧 → 落盘/送前端"
//! 这条链路从第一天就被验证，这里先提供一个 SMPTE 彩条测试帧——和电视
//! 检修图一个道理：**彩条显示正常，说明问题不在显示链路**。

/// 屏幕宽度（像素）。
pub const SCREEN_WIDTH: usize = 240;
/// 屏幕高度（像素）。
pub const SCREEN_HEIGHT: usize = 160;
/// 一帧的总字节数：宽 × 高 × 4 字节（RGBA）。
pub const FRAME_BYTES: usize = SCREEN_WIDTH * SCREEN_HEIGHT * 4;

/// 一帧完整画面。
///
/// 长度恒为 [`FRAME_BYTES`]——这个不变式不靠注释约定，而是由类型系统
/// 保证：`pixels` 字段私有，外部只能通过 `black()` / `test_pattern()`
/// 构造。M2 的 PPU 也会走同样的构造出口，想造出"半截帧"都过不了编译。
#[derive(Debug, Clone)]
pub struct Frame {
    pixels: Box<[u8]>,
}

impl Frame {
    /// 全黑帧。对应真机"强制空白"（DISPCNT 的 Force Blank 位）时的输出。
    pub fn black() -> Self {
        Self {
            pixels: vec![0u8; FRAME_BYTES].into(),
        }
    }

    /// SMPTE 风格彩条测试帧（M0 验证显示链路用）。
    ///
    /// 从左到右 8 条等宽竖条：白、黄、青、绿、品红、红、蓝、黑。
    /// 选这组颜色的原因：三原色、三间色加黑白全齐——显示通道接错
    /// （比如 R/B 互换）能一眼看出来。
    pub fn test_pattern() -> Self {
        const BARS: [[u8; 3]; 8] = [
            [0xFF, 0xFF, 0xFF], // 白
            [0xFF, 0xFF, 0x00], // 黄
            [0x00, 0xFF, 0xFF], // 青
            [0x00, 0xFF, 0x00], // 绿
            [0xFF, 0x00, 0xFF], // 品红
            [0xFF, 0x00, 0x00], // 红
            [0x00, 0x00, 0xFF], // 蓝
            [0x00, 0x00, 0x00], // 黑
        ];
        let mut frame = Self::black();
        for y in 0..SCREEN_HEIGHT {
            for x in 0..SCREEN_WIDTH {
                // x * 8 / 240：把横坐标等分成 8 段，选出所在彩条
                let [r, g, b] = BARS[x * BARS.len() / SCREEN_WIDTH];
                let i = (y * SCREEN_WIDTH + x) * 4;
                frame.pixels[i..i + 4].copy_from_slice(&[r, g, b, 0xFF]);
            }
        }
        frame
    }

    /// 只读像素视图（供 headless 落盘、将来 IPC 发送）。
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_frame_is_all_zero_and_correct_size() {
        let f = Frame::black();
        assert_eq!(f.pixels().len(), FRAME_BYTES);
        assert!(f.pixels().iter().all(|&b| b == 0));
    }

    #[test]
    fn test_pattern_bars_are_positioned_and_opaque() {
        let f = Frame::test_pattern();
        let px = f.pixels();
        // 第 0 行最左列应是第一条彩条"白"
        assert_eq!(&px[0..4], &[0xFF, 0xFF, 0xFF, 0xFF]);
        // 最右列应是最后一条"黑"
        let last = (SCREEN_WIDTH - 1) * 4;
        assert_eq!(&px[last..last + 4], &[0x00, 0x00, 0x00, 0xFF]);
        // 所有像素的 alpha 通道都必须不透明
        assert!(px.iter().skip(3).step_by(4).all(|&a| a == 0xFF));
    }
}
