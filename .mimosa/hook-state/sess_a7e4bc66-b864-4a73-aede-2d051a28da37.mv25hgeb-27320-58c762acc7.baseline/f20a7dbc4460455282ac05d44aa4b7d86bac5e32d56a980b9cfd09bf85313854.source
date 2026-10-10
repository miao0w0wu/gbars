//! 核心门面：把 CPU 与总线组装成一台"游戏机"。
//!
//! [`GameBoyAdvance`] 是外部世界（headless CLI、将来的 Tauri 后端）接触
//! 模拟器的唯一入口：装载 ROM → `step()` 逐条执行 → 读取状态/画面。
//! 随里程碑推进，这里会长出 `run_frame()`（M2）、按键注入（M3）、
//! 音频输出（M4）与即时存档（M6）。

use std::fs;
use std::path::Path;

use crate::bus::Bus;
use crate::cartridge::{Cartridge, CartridgeError};
use crate::cpu::{Cpu, CpuError};
use crate::frame::Frame;
use crate::scheduler::Scheduler;

/// 一台完整状态聚合在结构体内的 GBA（无全局变量，可整体快照/恢复）。
pub struct GameBoyAdvance {
    pub cpu: Cpu,
    pub bus: Bus,
    pub scheduler: Scheduler,
    /// 最新一次 [`GameBoyAdvance::run_frame`] 输出的帧。
    frame: Frame,
}

impl GameBoyAdvance {
    pub fn load_rom(path: impl AsRef<Path>) -> Result<Self, CartridgeError> {
        Self::load_rom_bytes(fs::read(path)?.into())
    }

    /// 从内存中的 ROM 字节构造（测试与将来"从容器/网络加载"用）。
    pub fn load_rom_bytes(rom: Box<[u8]>) -> Result<Self, CartridgeError> {
        let bus = Bus::new(Cartridge::from_bytes(rom)?);
        let mut gba = Self {
            cpu: Cpu::new_no_bios(),
            bus,
            scheduler: Scheduler::new(),
            frame: Frame::black(),
        };
        gba.apply_no_bios_io_defaults();
        Ok(gba)
    }

    /// 无 BIOS 启动时，真机 BIOS 在把控制权交给 ROM 前已摆好的 I/O 默认值
    /// （GBATEK "BIOS"；数值与 docs/design.md §6.3 一致）。
    fn apply_no_bios_io_defaults(&mut self) {
        self.bus.write_u16(0x0400_0000, 0x0080); // DISPCNT：强制空白（不显示）
        self.bus.write_u16(0x0400_0204, 0x4317); // WAITCNT：常规等待状态
        self.bus.write_u16(0x0400_0208, 0x0000); // IME = 0：中断总开关关闭
    }

    pub fn cart(&self) -> &Cartridge {
        self.bus.cart()
    }

    pub fn ppu(&self) -> &crate::ppu::Ppu {
        self.bus.ppu()
    }

    /// 执行一条指令。
    pub fn step(&mut self) -> Result<(), CpuError> {
        self.cpu.step(&mut self.bus)
    }

    /// 推进一整帧（280,896 周期），返回最新画面。
    pub fn run_frame(&mut self) -> Result<&Frame, CpuError> {
        self.scheduler.reset_frame();
        self.scheduler.run_frame(&mut self.cpu, &mut self.bus)?;
        self.capture_frame();
        Ok(&self.frame)
    }

    /// 把 PPU 的当前画面拷进帧缓冲。
    fn capture_frame(&mut self) {
        let ppu = self.bus.ppu();
        let frame = &mut self.frame;
        let pixels = frame.pixels_mut();
        for y in 0..crate::frame::SCREEN_HEIGHT {
            ppu.write_line_to_frame(y, pixels);
        }
    }

    /// 最近一帧（在 [`GameBoyAdvance::run_frame`] 之前是全黑）。
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手工汇编的求和程序：r0 = 5+4+3+2+1 = 15，计数器逐个存入 EWRAM。
    ///
    /// 这是"取指 → 译码 → 执行 → 内存"完整闭环的第一个端到端证据。
    /// 逐条指令的编码拆解见 docs/notes/01 §2.4。
    const PROGRAM: [u32; 9] = [
        0xE3A0_0000, // 0x08000000: MOV  r0, #0            ；和 = 0
        0xE3A0_1005, // 0x08000004: MOV  r1, #5            ；计数器 = 5
        0xE3A0_2402, // 0x08000008: MOV  r2, #0x02000000   ；EWRAM 基址（立即数旋转）
        0xE080_0001, // 0x0800000C: ADD  r0, r0, r1        ← 循环体
        0xE482_1004, // 0x08000010: STR  r1, [r2], #4      ；存计数器，指针后移
        0xE251_1001, // 0x08000014: SUBS r1, r1, #1        ；计数器 -1，更新标志
        0x1AFF_FFFB, // 0x08000018: BNE  循环体（偏移 -5×4 = 回到 0x0C 的 ADD）
        0xE582_0000, // 0x0800001C: STR  r0, [r2]          ；存最终和
        0xEAFF_FFFE, // 0x08000020: B    .（自旋等待断言）
    ];

    #[test]
    fn sum_program_runs_end_to_end() {
        // 头部全 0 的 256 字节迷你 ROM（≥192 字节头部即可装载）
        let mut rom = vec![0u8; 0x100];
        for (i, word) in PROGRAM.iter().enumerate() {
            rom[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let mut gba = GameBoyAdvance::load_rom_bytes(rom.into()).unwrap();

        // 3 条初始化 + 5 轮循环（每轮 4 条）+ 2 条收尾 = 25 条指令
        for _ in 0..25 {
            gba.step().unwrap();
        }

        assert_eq!(gba.cpu.reg(0), 15); // 5+4+3+2+1
        assert_eq!(gba.cpu.reg(1), 0); // 计数器归零
        assert_eq!(gba.cpu.reg(2), 0x0200_0014); // r2 被后索引推进 5 次（0+5×4）
        // STR r1,[r2],#4 把 5..1 依次写入 EWRAM
        assert_eq!(gba.bus.read_u32(0x0200_0000), 5);
        assert_eq!(gba.bus.read_u32(0x0200_000C), 2);
        assert_eq!(gba.bus.read_u32(0x0200_0010), 1);
        assert_eq!(gba.bus.read_u32(0x0200_0014), 15); // 收尾的 STR r0
        // 停在自旋分支上不再前进（regs[15] = 自旋地址 + 8）
        assert_eq!(gba.cpu.reg(15), 0x0800_0028);
        assert_eq!(gba.cpu.steps(), 25);
    }

    /// 手工汇编的 Thumb 程序：把 ARM 求和程序写成 Thumb，并额外演示
    /// 函数调用（BL → POP {PC} 返回）与栈帧——这正是 GBA 游戏的日常形态。
    ///
    /// 程序：从入口 BL 调用 sum5 子程序；sum5 用 PUSH 保存 LR、循环累加
    /// 5..1、把结果写入 EWRAM，再 POP {PC} 返回。
    /// 逐条指令的编码拆解见 docs/notes/02 §2.5。
    const THUMB_PROGRAM: [u16; 28] = [
        // 入口（0x08000010，索引 0）
        0xF000, 0xF80E, // BL sum5          ← 目标 0x08000030（见下方布局）
        0xE7FE, // B .（自旋等待断言）
        // 填充到 0x08000030（索引 3..=15，13 个半字）
        0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
        0x0000, 0x0000,
        // sum5 子程序（0x08000030，索引 16）
        0xB501, // PUSH {r0, lr}    ；保存现场：r0 作累加器初值，lr 是返回点
        0x2000, // MOV  r0, #0      ；和 = 0
        0x2105, // MOV  r1, #5      ；计数器 = 5
        // 循环体（0x08000036，索引 19）
        0x1808, // ADD  r0, r1, r0  ；r0 = r0 + r1
        0x3901, // SUB  r1, #1      ；计数器 -1（更新标志）
        0xD1FC, // BNE  循环体       ；Z=0 跳回（偏移 -4×2 = -8）
        0x4901, // LDR  r1, [pc, #4]；LDR 在 0x0800003C，可见 PC = 0x08000040
        0x6008, // STR  r0, [r1]    ；存最终和
        0xBD01, // POP  {r0, pc}    ；恢复现场并返回（装载 PC = 返回）
        0x0000, // 填充（0x08000042）
        0x0000,
        0x0200, // 常数池（0x08000044 = 可见PC+4）：0x02000000 = EWRAM 基址
                // ↑ 低半字在前：0x0000 | (0x0200 << 16) = 0x0200_0000
    ];

    #[test]
    fn thumb_program_with_call_and_stack() {
        // 组装 ROM：开头一条 ARM 指令（B 到 Thumb 代码）不可省——真机
        // 从 0x08000000 以 ARM 态起步，必须先切到 Thumb。
        let mut rom = vec![0u8; 0x200];
        // ARM 引导段：真机从 0x08000000 以 **ARM 态**起步，必须先切到 Thumb。
        // 0x08000000: LDR r0, [pc, #4] —— PC 可见值 = 指令地址 + 8 = 0x08000008，
        //             再加 4 = 0x0800000C，那里存着 Thumb 入口地址
        rom[0..4].copy_from_slice(&0xE59F_0004u32.to_le_bytes());
        // 0x08000004: BX r0（按目标位 0 切到 Thumb）
        rom[4..8].copy_from_slice(&0xE12F_FF10u32.to_le_bytes());
        // 0x08000008: 占位（保持 4 字节对齐）
        rom[8..12].copy_from_slice(&0x0000_0000u32.to_le_bytes());
        // 0x0800000C: 常数 = Thumb 入口地址 0x08000010 | 1（位 0 = 1 表示 Thumb）
        rom[12..16].copy_from_slice(&0x0800_0011u32.to_le_bytes());
        // 0x08000010 起放置 Thumb 程序
        for (i, half) in THUMB_PROGRAM.iter().enumerate() {
            rom[0x10 + i * 2..0x10 + i * 2 + 2].copy_from_slice(&half.to_le_bytes());
        }

        let mut gba = GameBoyAdvance::load_rom_bytes(rom.into()).unwrap();
        // 步数留足：2 条 ARM（LDR+BX）+ 入口 3 条 + 子程序 9 条 + 循环 5 轮 ×3
        for _ in 0..60 {
            gba.step().unwrap();
        }

        // 子程序返回后，入口的 B .（自旋）仍在 Thumb 态——BL 留下的 LR
        // 带着 Thumb 位（|1），POP {PC} 装载后按位 0 维持 Thumb。
        assert!(gba.cpu.t(), "POP PC 装载带位 1 的返回地址应保持 Thumb 态");
        assert_eq!(gba.cpu.reg(0), 0x0800_0011); // r0 被 POP {r0,pc} 恢复为入口常数
        assert_eq!(
            gba.bus.read_u32(0x0200_0000),
            15,
            "Thumb 子程序算出 5+4+3+2+1（r1 应指向 EWRAM 基址）"
        );
        // 栈已平衡（PUSH/POP 成对）
        assert_eq!(gba.cpu.reg(13), 0x0300_7F00);
    }
}
