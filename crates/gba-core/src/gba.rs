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

/// 一台完整状态聚合在结构体内的 GBA（无全局变量，可整体快照/恢复）。
pub struct GameBoyAdvance {
    pub cpu: Cpu,
    pub bus: Bus,
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

    /// 执行一条指令（M2 会变成 `run_frame()` 整帧推进）。
    pub fn step(&mut self) -> Result<(), CpuError> {
        self.cpu.step(&mut self.bus)
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
}
