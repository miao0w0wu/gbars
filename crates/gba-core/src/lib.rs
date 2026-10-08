//! # gba-core —— Game Boy Advance 模拟核心
//!
//! 纯 Rust 的 GBA 硬件模拟库，是整个模拟器的"发动机"。
//!
//! ## 设计原则（详见 `docs/design.md` §4.2）
//!
//! - **无 UI 依赖**：本 crate 不依赖 Tauri、窗口或音频库，
//!   可在任何环境（包括 CI 与单元测试）中独立运行；
//! - **帧进帧出**：外界只送入按键状态，取走视频帧与音频采样，
//!   不直接触碰内部硬件状态；
//! - **单一时钟**：所有硬件由统一的调度器（M2 引入）按 GBA 主频
//!   16.777216 MHz 推进，杜绝"各设备各数各的"。
//!
//! ## 模块地图（随里程碑逐步充实）
//!
//! | 模块 | 里程碑 | 职责 |
//! |------|--------|------|
//! | [`cartridge`] | M0 | ROM 装载与头部解析 |
//! | [`frame`] | M0 | 视频帧输出契约（240×160 RGBA8888） |
//! | `cpu` | M1 | ARM7TDMI 解释器（ARM + Thumb 双指令集） |
//! | `bus` | M1 | 内存总线与地址映射（CPU 与硬件的"主干道"） |
//! | `bios` | M1 | 无 BIOS 启动状态 + SWI 高层模拟 |
//! | `ppu` | M2 | 图形渲染（背景/精灵/窗口/混合） |
//! | `dma` / `timer` / `interrupts` | M3 | DMA、定时器、中断 |
//! | `savedata` | M3 | SRAM / Flash / EEPROM 存档 |
//! | `apu` | M4 | 音频（PSG 四通道 + DirectSound FIFO） |

pub mod cartridge;
pub mod frame;
