//! ARM7TDMI CPU 核心：寄存器组、运行模式、异常入口与统一执行接口。
//!
//! # 硬件背景：ARM7TDMI
//!
//! GBA 的主控 CPU，ARMv4T 架构（"T" = 支持 Thumb 16 位指令集），
//! 主频 16.777216 MHz。要点：
//!
//! - **16 个通用寄存器** r0–r15（r13 惯用作栈指针 SP、r14 链接寄存器 LR、
//!   r15 程序计数器 PC）+ 1 个 CPSR（当前程序状态寄存器：N/Z/C/V 标志、
//!   IRQ/FIQ 屏蔽、ARM/Thumb 切换位 T、低 5 位运行模式）；
//! - **运行模式与寄存器银行**：共 7 种模式。进入异常模式时，r13/r14（FIQ
//!   还有 r8–r12）会"换一组"——每组叫一个银行（bank），被换出的原值不丢，
//!   返回时换回来。这保证中断处理程序不会踩碎游戏的寄存器现场；
//! - **三级流水线**：取指 → 译码 → 执行三条指令同时在进行，因此"读 PC"
//!   得到的是当前指令地址 + 8（ARM 态）。本实现把这一点做成了硬约定，
//!   见 [`Cpu`]\[`Cpu::regs`\] 的文档。
//!
//! # 本实现的 PC 约定（重要）
//!
//! 执行指令期间，`regs[15]` **恒等于"此时读 R15 硬件会给出的值"**：
//!
//! ```text
//! ARM   态：regs[15] = 当前指令地址 + 8    （取指地址 = regs[15] - 8）
//! Thumb 态：regs[15] = 当前指令地址 + 4    （取指地址 = regs[15] - 4）
//! ```
//!
//! 因此指令内所有"读 PC"都直接用 `regs[15]`，零调整；而分支类指令
//! 落地时要把"目标地址 + 8/+4"写回 `regs[15]`（保持约定），普通指令
//! 执行完 `regs[15] += 4/+2` 推进流水线。
//!
//! # 周期计量的现状
//!
//! 本阶段只有 [`Cpu::steps`]（指令条数）计数。精确到等待状态的周期模型
//! 需要统一调度器配合，在 M2 一并引入（见 docs/design.md §6.2）。

pub(crate) mod alu;
pub(crate) mod isa_arm;

use crate::bus::Bus;

/// 处理器运行模式（CPSR 低 5 位）。GBATEK "ARM7TDMI Overview"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    User = 0x10,
    Fiq = 0x11,
    Irq = 0x12,
    Supervisor = 0x13,
    Abort = 0x17,
    Undefined = 0x18,
    System = 0x1F,
}

impl Mode {
    pub fn from_bits(bits: u32) -> Option<Mode> {
        Some(match bits {
            0x10 => Mode::User,
            0x11 => Mode::Fiq,
            0x12 => Mode::Irq,
            0x13 => Mode::Supervisor,
            0x17 => Mode::Abort,
            0x18 => Mode::Undefined,
            0x1F => Mode::System,
            _ => return None,
        })
    }

    /// 银行编号：r13/r14/SPSR 按此分组存放（见模块文档"寄存器银行"）。
    /// User 与 System 共享一组是架构规定——它们本就是同一种"普通"状态。
    pub(crate) fn bank(self) -> usize {
        match self {
            Mode::User | Mode::System => 0,
            Mode::Fiq => 1,
            Mode::Irq => 2,
            Mode::Supervisor => 3,
            Mode::Abort => 4,
            Mode::Undefined => 5,
        }
    }

    /// 只有异常模式才有 SPSR（保存进入异常那一刻的 CPSR）。
    pub(crate) fn has_spsr(self) -> bool {
        !matches!(self, Mode::User | Mode::System)
    }
}

/// CPU 执行错误：只用于"开发期尚未实现的合法指令"。
/// 硬件有定义的行为（如未定义指令触发 UDF 异常）不走这里，
/// 走 [`Cpu::enter_exception`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuError {
    Unimplemented { insn: u32, addr: u32 },
}

impl CpuError {
    pub(crate) fn unimplemented(insn: u32, addr: u32) -> Self {
        CpuError::Unimplemented { insn, addr }
    }
}

impl std::fmt::Display for CpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CpuError::Unimplemented { insn, addr } => {
                write!(f, "未实现的指令 {insn:#010x} @ {addr:#010x}")
            }
        }
    }
}

impl std::error::Error for CpuError {}

/// 条件码判定：ARM 每条指令都带 4 位条件字段，先判定再执行。
/// 一条 `MOVEQ` 不满足条件时**不执行也不报错**，只消耗一个流水线周期。
pub(crate) fn cond_pass(cond: u32, cpsr: u32) -> bool {
    let n = cpsr & (1 << 31) != 0;
    let z = cpsr & (1 << 30) != 0;
    let c = cpsr & (1 << 29) != 0;
    let v = cpsr & (1 << 28) != 0;
    match cond {
        0x0 => z,            // EQ：相等
        0x1 => !z,           // NE
        0x2 => c,            // CS/HS：无符号 ≥
        0x3 => !c,           // CC/LO：无符号 <
        0x4 => n,            // MI：负
        0x5 => !n,           // PL
        0x6 => v,            // VS：溢出
        0x7 => !v,           // VC
        0x8 => c && !z,      // HI：无符号 >
        0x9 => !c || z,      // LS：无符号 ≤
        0xA => n == v,       // GE：有符号 ≥
        0xB => n != v,       // LT：有符号 <
        0xC => !z && n == v, // GT：有符号 >
        0xD => z || n != v,  // LE：有符号 ≤
        _ => true,           // AL（NV 由调用方单独排除）
    }
}

/// ARM7TDMI 的完整可编程状态。
pub struct Cpu {
    /// 当前可见的 r0–r15（r13=SP、r14=LR、r15=PC）。
    ///
    /// **流水线约定**：执行指令期间 `regs[15]` = 当前指令地址 + 8（ARM 态）
    /// / + 4（Thumb 态），即"读 R15 的硬件可见值"；取指地址 = `regs[15] - 8`
    /// （ARM 态）。分支落地时写入"目标地址 + 8/+4"以维持该约定。
    pub(crate) regs: [u32; 16],
    pub(crate) cpsr: u32,
    /// FIQ 独享的 r8–r12。进入/退出 FIQ 时与 `regs[8..13]` 整体交换。
    pub(crate) fiq_regs: [u32; 5],
    /// r13 按模式的银行副本（索引 = [`Mode::bank`]）。
    pub(crate) banked_sp: [u32; 6],
    /// r14 按模式的银行副本。
    pub(crate) banked_lr: [u32; 6],
    /// SPSR 按模式存放（User/System 槽位恒 0，不可用）。
    pub(crate) spsr: [u32; 6],
    /// 本条指令是否改写了 PC（分支/LDR PC/异常入口置位）。
    pub(crate) pc_written: bool,
    /// Halt/IntrWait 的睡眠标志（唤醒语义随 M3 中断实现）。
    pub(crate) halted: bool,
    /// 已执行的指令条数（周期级时序模型随 M2 调度器引入）。
    pub(crate) steps: u64,
}

impl Cpu {
    /// 无 BIOS 启动状态：把 CPU 摆成"真机 BIOS 刚交接完"的样子。
    /// 依据 GBATEK "BIOS" 章与 docs/design.md §6.3。
    pub fn new_no_bios() -> Self {
        let mut cpu = Self {
            regs: [0; 16],
            cpsr: Mode::System as u32, // 0x1F：System 模式，IRQ/FIQ 未屏蔽
            fiq_regs: [0; 5],
            banked_sp: [0; 6],
            banked_lr: [0; 6],
            spsr: [0; 6],
            pc_written: false,
            halted: false,
            steps: 0,
        };
        // regs[15] = 启动地址 + 8：按流水线约定，"即将执行 0x08000000"
        cpu.regs[15] = 0x0800_0008;
        // 真机 BIOS 交接前摆好的三个栈指针
        cpu.banked_sp[Mode::User.bank()] = 0x0300_7F00; // usr/sys
        cpu.banked_sp[Mode::Irq.bank()] = 0x0300_7FA0; // irq
        cpu.banked_sp[Mode::Supervisor.bank()] = 0x0300_7FE0; // svc
        // 当前可见 SP 要加载银行里的值（银行只是副本，可见值在 regs[13]）
        cpu.regs[13] = cpu.banked_sp[Mode::User.bank()];
        cpu
    }

    // ---- 对外只读视图（headless 调试与测试用）----

    pub fn reg(&self, n: usize) -> u32 {
        self.regs[n]
    }

    pub fn cpsr(&self) -> u32 {
        self.cpsr
    }

    /// 当前模式。模式位由 [`Cpu::write_cpsr`] 规范化维护，不会出现非法值。
    pub fn mode(&self) -> Mode {
        Mode::from_bits(self.cpsr & 0x1F).expect("CPSR 中出现非法模式位")
    }

    /// 是否处于 Thumb 状态（CPSR 位 5）。
    pub fn t(&self) -> bool {
        self.cpsr & (1 << 5) != 0
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    // ---- 标志位操作（供 isa_arm 使用）----

    pub(crate) fn carry(&self) -> bool {
        self.cpsr & (1 << 29) != 0
    }

    pub(crate) fn set_nz(&mut self, result: u32) {
        self.set_flag_n(result >> 31 == 1);
        self.set_flag_z(result == 0);
    }

    pub(crate) fn set_flag_n(&mut self, n: bool) {
        self.cpsr = with_bit(self.cpsr, 31, n);
    }

    pub(crate) fn set_flag_z(&mut self, z: bool) {
        self.cpsr = with_bit(self.cpsr, 30, z);
    }

    pub(crate) fn set_c(&mut self, c: bool) {
        self.cpsr = with_bit(self.cpsr, 29, c);
    }

    pub(crate) fn set_v(&mut self, v: bool) {
        self.cpsr = with_bit(self.cpsr, 28, v);
    }

    // ---- 模式与状态切换 ----

    /// 切换运行模式并完成寄存器银行交换；相同模式为空操作。
    pub(crate) fn switch_mode(&mut self, new: Mode) {
        let old = self.mode();
        if old == new {
            return;
        }
        // FIQ 的 r8–r12 独享：进出 FIQ 时与当前可见寄存器整体互换
        if (old == Mode::Fiq) != (new == Mode::Fiq) {
            self.regs[8..13].swap_with_slice(&mut self.fiq_regs);
        }
        let (ob, nb) = (old.bank(), new.bank());
        if ob != nb {
            self.banked_sp[ob] = self.regs[13];
            self.banked_lr[ob] = self.regs[14];
            self.regs[13] = self.banked_sp[nb];
            self.regs[14] = self.banked_lr[nb];
        }
        self.cpsr = (self.cpsr & !0x1F) | (new as u32);
    }

    /// 整体写入 CPSR（MSR / SPSR 恢复路径）：处理模式切换。
    /// 写入非法模式位在真机上是不可预测行为；这里规范化为保持原模式。
    pub(crate) fn write_cpsr(&mut self, value: u32) {
        let target = Mode::from_bits(value & 0x1F).unwrap_or_else(|| self.mode());
        self.switch_mode(target);
        self.cpsr = (value & !0x1F) | (target as u32);
    }

    /// 异常入口（GBATEK "Exceptions"）。SWI/未定义指令共用：
    /// LR = 当前指令地址 + 4；SPSR 保存旧 CPSR；切回 ARM 态并屏蔽 IRQ；
    /// PC 跳到向量表。IRQ/FIQ 入口在 M3 中断控制器就绪后复用这里。
    pub(crate) fn enter_exception(&mut self, vector: u32, new_mode: Mode) {
        let old_cpsr = self.cpsr;
        let lr = self.regs[15].wrapping_sub(4);
        self.switch_mode(new_mode);
        self.spsr[new_mode.bank()] = old_cpsr;
        self.regs[14] = lr;
        self.cpsr = (self.cpsr | (1 << 7)) & !(1 << 5); // I=1，T=0
        if new_mode == Mode::Fiq {
            self.cpsr |= 1 << 6; // FIQ 连 F 一起屏蔽
        }
        // 流水线约定：即将执行的指令在 vector，故 regs[15] = vector + 8
        self.regs[15] = vector + 8;
        self.pc_written = true;
    }

    /// 取指并执行一条指令。
    pub fn step(&mut self, bus: &mut Bus) -> Result<(), CpuError> {
        if self.halted {
            return Ok(()); // 睡眠唤醒随 M3 中断实现
        }
        if self.t() {
            // Thumb 执行在 M1-b 加入；撞到先报"未实现"而非静默出错
            return Err(CpuError::unimplemented(0, self.regs[15].wrapping_sub(4)));
        }
        let insn = bus.read_u32(self.regs[15].wrapping_sub(8));
        self.arm_step(insn, bus)
    }
}

fn with_bit(v: u32, bit: u32, on: bool) -> u32 {
    if on { v | (1 << bit) } else { v & !(1 << bit) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_switch_banks_r13_r14() {
        let mut cpu = Cpu::new_no_bios();
        cpu.regs[13] = 0x1111_1111; // usr/sys 的 SP
        cpu.switch_mode(Mode::Supervisor);
        assert_eq!(cpu.regs[13], 0x0300_7FE0); // svc 栈从银行加载
        cpu.regs[13] = 0x2222_2222;
        cpu.regs[14] = 0x3333_3333;
        cpu.switch_mode(Mode::User);
        assert_eq!(cpu.regs[13], 0x1111_1111); // 回到 usr，原 SP 恢复
        assert_eq!(cpu.cpsr & 0x1F, Mode::User as u32);
    }

    #[test]
    fn fiq_banks_r8_to_r12() {
        let mut cpu = Cpu::new_no_bios();
        cpu.regs[8] = 0xAA;
        cpu.switch_mode(Mode::Fiq);
        assert_eq!(cpu.regs[8], 0); // FIQ 有自己的一组 r8–r12
        cpu.regs[8] = 0xBB;
        cpu.switch_mode(Mode::Irq);
        assert_eq!(cpu.regs[8], 0xAA); // 回到普通组
        assert_eq!(cpu.cpsr & 0x1F, Mode::Irq as u32);
    }

    #[test]
    fn cond_codes_match_gbatek() {
        let z = 1 << 30;
        assert!(cond_pass(0x0, z)); // EQ
        assert!(!cond_pass(0x1, z)); // NE
        assert!(cond_pass(0x9, z)); // LS：!C || Z
        assert!(!cond_pass(0x8, z)); // HI：C && !Z
        let nv = 1 << 31; // N=1, V=0 → 有符号小于
        assert!(cond_pass(0xB, nv)); // LT
        assert!(!cond_pass(0xA, nv)); // GE
    }
}
