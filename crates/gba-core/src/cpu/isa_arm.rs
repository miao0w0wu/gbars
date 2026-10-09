//! ARM 指令集执行（ARMv4T）。
//!
//! # 硬件背景：ARM 指令编码
//!
//! 每条 ARM 指令固定 32 位，高 4 位是条件码，位 27–25 决定大类
//! （GBATEK "ARM Instruction Set" 的编码地图）：
//!
//! ```text
//! 31    28 27  25                0
//! ┌───────┬──────┬─────────────────────┐
//! │ cond  │ 000  │ 数据处理/乘法/杂项   │ ← 一个大类里还要细辨
//! │ cond  │ 001  │ 数据处理（立即数）   │
//! │ cond  │ 010  │ LDR/STR（单数据传输）│
//! │ cond  │ 011  │ 协处理器 → GBA 上必触发 UDF
//! │ cond  │ 100  │ LDM/STM（块传输，M1-b）
//! │ cond  │ 101  │ B/BL（分支）
//! │ cond  │ 110  │ 协处理器传输 → UDF
//! │ cond  │ 111  │ SWI（位 24=1）/ 协处理器（=0）
//! └───────┴──────┴─────────────────────┘
//! ```
//!
//! 实现策略：**直接从原始指令字按位提取字段执行**（不做中间译码结构）。
//! 优点是编码格式本身就是教学内容、代码量最小；等 Thumb 加入后若执行
//! 分发成为性能热点，再考虑译码/执行分离（mGBA 的做法）。
//!
//! 本子步覆盖：数据处理（全 16 操作 + 桶形移位）、B/BL/BX、LDR/STR、
//! MUL/MLA/长乘法、MRS/MSR、SWI/UDF 异常入口。
//! 待 M1-b：LDM/STM、SWP、半字/有符号传输；待 M1-d：SWI 的 HLE。

use super::{Cpu, CpuError, Mode, alu, cond_pass};
use crate::bus::Bus;

/// 向量表地址（GBATEK "Exceptions"）
const VECTOR_UNDEFINED: u32 = 0x04;
const VECTOR_SWI: u32 = 0x08;

impl Cpu {
    /// 执行一条 ARM 指令。`insn` 必须已从 `regs[15] - 8` 取出。
    pub(crate) fn arm_step(&mut self, insn: u32, bus: &mut Bus) -> Result<(), CpuError> {
        self.pc_written = false;
        let cond = insn >> 28;
        // 0xE = AL 恒执行；0xF = NV 在 ARMv4 恒不执行（当条件不满足跳过）
        if cond != 0xF && cond_pass(cond, self.cpsr) {
            self.execute(insn, bus)?;
        }
        if !self.pc_written {
            self.regs[15] = self.regs[15].wrapping_add(4);
        }
        self.steps += 1;
        Ok(())
    }

    /// 按位 27–25 粗分类分发（见模块文档的编码地图）。
    fn execute(&mut self, insn: u32, bus: &mut Bus) -> Result<(), CpuError> {
        match (insn >> 25) & 0x7 {
            0b000 => self.category_000(insn),
            0b001 => self.data_processing(insn), // 立即数版（含 MSR 立即数）
            0b010 => self.single_transfer(insn, bus),
            0b011 | 0b110 => {
                // 协处理器空间：GBA 没有游戏可用的协处理器，真机行为
                // 就是未定义指令异常
                self.enter_exception(VECTOR_UNDEFINED, Mode::Undefined);
                Ok(())
            }
            0b100 => Err(CpuError::unimplemented(insn, self.regs[15].wrapping_sub(8))), // LDM/STM：M1-b
            0b101 => {
                self.branch(insn);
                Ok(())
            }
            _ => {
                if insn & (1 << 24) != 0 {
                    self.swi(); // 1111 + 位24=1：SWI
                } else {
                    self.enter_exception(VECTOR_UNDEFINED, Mode::Undefined); // CDP/MCR
                }
                Ok(())
            }
        }
    }

    /// 位 27–25 = 000 的杂烩区：数据处理、BX、乘法、SWP、半字传输。
    /// （这一类都不直接访问内存，无须总线。）
    fn category_000(&mut self, insn: u32) -> Result<(), CpuError> {
        // BX 占据 cond 0001 0010 1111 1111 0001 xxxx 的编码空间
        if insn & 0x0FFF_FFF0 == 0x012F_FF10 {
            self.branch_exchange(insn);
            return Ok(());
        }
        // 位 7、4 同时为 1 → 乘法 / SWP / 半字传输 的判别区
        if insn & 0x90 == 0x90 {
            if insn & (1 << 24) == 0 {
                self.multiply(insn); // 位 27-24 = 0000：乘法族
                return Ok(());
            }
            // SWP 与半字/有符号传输：M1-b 实现（先报未实现）
            return Err(CpuError::unimplemented(insn, self.regs[15].wrapping_sub(8)));
        }
        self.data_processing(insn)
    }

    /// 数据处理：16 种操作 × 立即数/移位寄存器操作数 × 可选 S 标志更新。
    fn data_processing(&mut self, insn: u32) -> Result<(), CpuError> {
        // MRS/MSR 潜伏在比较类指令 + S=0 的"非法编码"空间里，必须
        // 先于普通数据处理识别（掩码与 GBATEK 一致）
        if insn & 0x0FBF_0FFF == 0x010F_0000 {
            self.mrs(insn);
            return Ok(());
        }
        if insn & 0x0DB0_F000 == 0x0120_F000 {
            self.msr(insn);
            return Ok(());
        }

        let op = (insn >> 21) & 0xF;
        let s = insn & (1 << 20) != 0;
        let rn = ((insn >> 16) & 0xF) as usize;
        let rd = ((insn >> 12) & 0xF) as usize;

        let a = self.regs[rn]; // Rn=PC 时即"指令地址+8"，正是架构规定
        let (b, shifter_carry) = self.operand2(insn);

        // (结果, 可选 C, 可选 V)：算术类产 C/V；逻辑类 C 来自移位器
        let (result, c_out, v_out): (u32, Option<bool>, Option<bool>) = match op {
            0x0 => (a & b, Some(shifter_carry), None), // AND
            0x1 => (a ^ b, Some(shifter_carry), None), // EOR
            0x2 | 0xA => {
                // SUB / CMP
                let (r, c, v) = alu::sub_with_borrow(a, b, false);
                (r, Some(c), Some(v))
            }
            0x3 => {
                // RSB（反向减法：b - a）
                let (r, c, v) = alu::sub_with_borrow(b, a, false);
                (r, Some(c), Some(v))
            }
            0x4 | 0xB => {
                // ADD / CMN
                let (r, c, v) = alu::add_with_carry(a, b, false);
                (r, Some(c), Some(v))
            }
            0x5 => {
                // ADC
                let (r, c, v) = alu::add_with_carry(a, b, self.carry());
                (r, Some(c), Some(v))
            }
            0x6 => {
                // SBC
                let (r, c, v) = alu::sub_with_borrow(a, b, !self.carry());
                (r, Some(c), Some(v))
            }
            0x7 => {
                // RSC
                let (r, c, v) = alu::sub_with_borrow(b, a, !self.carry());
                (r, Some(c), Some(v))
            }
            0x8 => (a & b, Some(shifter_carry), None), // TST
            0x9 => (a ^ b, Some(shifter_carry), None), // TEQ
            0xC => (a | b, Some(shifter_carry), None), // ORR
            0xD => (b, Some(shifter_carry), None),     // MOV
            0xE => (a & !b, Some(shifter_carry), None), // BIC
            _ => (!b, Some(shifter_carry), None),      // MVN
        };

        if s {
            self.set_nz(result);
            if let Some(c) = c_out {
                self.set_c(c);
            }
            if let Some(v) = v_out {
                self.set_v(v);
            }
        }
        // 比较类（op 0x8–0xB）只影响标志，不写回
        if matches!(op, 0x8..=0xB) {
            return Ok(());
        }
        if rd == 15 {
            self.write_pc_arm(result, s); // S=1 写 PC = 异常返回（恢复 CPSR）
        } else {
            self.regs[rd] = result;
        }
        Ok(())
    }

    /// 数据处理第二操作数：立即数（可旋转）或（可移位的）寄存器。
    /// 返回 (值, 移位器进位输出)。
    fn operand2(&mut self, insn: u32) -> (u32, bool) {
        let carry_in = self.carry();
        if insn & (1 << 25) != 0 {
            // 立即数：8 位值 ROR (rot*2)；rot=0 → 不旋转、进位不变（GBATEK）
            let rot = ((insn >> 8) & 0xF) * 2;
            if rot == 0 {
                (insn & 0xFF, carry_in)
            } else {
                alu::rotate_imm_carry(insn & 0xFF, rot)
            }
        } else {
            let rm = (insn & 0xF) as usize;
            let ty = alu::ShiftType::from((insn >> 5) & 3);
            if insn & (1 << 4) == 0 {
                // 立即数移位量（5 位字段）
                alu::shift_imm(self.regs[rm], ty, (insn >> 7) & 0x1F, carry_in)
            } else {
                // 寄存器移位：移位量 = Rs 低 8 位（此时位 7 必为 0，
                // 否则会落进 category_000 的乘法判别区）
                let rs = ((insn >> 8) & 0xF) as usize;
                let amount = self.regs[rs] & 0xFF;
                alu::shift_reg(self.regs[rm], ty, amount, carry_in)
            }
        }
    }

    /// MRS Rd, CPSR|SPSR：读状态寄存器。
    fn mrs(&mut self, insn: u32) {
        let rd = ((insn >> 12) & 0xF) as usize;
        let spsr = insn & (1 << 22) != 0;
        self.regs[rd] = if spsr {
            if self.mode().has_spsr() {
                self.spsr[self.mode().bank()]
            } else {
                0 // User 态读 SPSR：真机不可预测，这里返回 0
            }
        } else {
            self.cpsr
        };
    }

    /// MSR CPSR|SPSR_<field>, Rm|#imm：按字节掩码写状态寄存器。
    fn msr(&mut self, insn: u32) {
        let mask = psr_field_mask((insn >> 16) & 0xF);
        let spsr = insn & (1 << 22) != 0;
        let value = if insn & (1 << 25) != 0 {
            // 立即数操作数：同样的 8 位 + ROR 编码（这里进位无意义）
            let rot = ((insn >> 8) & 0xF) * 2;
            (insn & 0xFF).rotate_right(rot)
        } else {
            self.regs[(insn & 0xF) as usize]
        };
        if spsr {
            if self.mode().has_spsr() {
                let b = self.mode().bank();
                self.spsr[b] = (self.spsr[b] & !mask) | (value & mask);
            }
        } else {
            let new = (self.cpsr & !mask) | (value & mask);
            self.write_cpsr(new);
        }
    }

    /// 把结果写进 PC 的统一路径（数据处理写 Rd=PC、LDR PC）。
    /// `restore_spsr`：MOVS PC 等"异常返回"——同时从 SPSR 恢复 CPSR。
    pub(crate) fn write_pc_arm(&mut self, value: u32, restore_spsr: bool) {
        if restore_spsr && self.mode().has_spsr() {
            let saved = self.spsr[self.mode().bank()];
            self.write_cpsr(saved);
        }
        // 维持流水线约定：执行目标指令时 regs[15] = 目标 + 8（ARM）/ +4（Thumb，
        // SPSR 恢复可能带回 T=1，即 v4 的异常返回状态切换）
        if self.t() {
            self.regs[15] = (value & !1).wrapping_add(4);
        } else {
            self.regs[15] = (value & !3).wrapping_add(8);
        }
        self.pc_written = true;
    }

    /// B/BL：24 位补码偏移 × 4，相对"本指令 + 8"。
    fn branch(&mut self, insn: u32) {
        let offset = alu::sign_extend(insn & 0x00FF_FFFF, 24) << 2;
        if insn & (1 << 24) != 0 {
            // BL：LR = 本指令地址 + 4（返回点；硬件为对齐流水线多减了 4）
            self.regs[14] = self.regs[15].wrapping_sub(4);
        }
        let target = self.regs[15].wrapping_add(offset as u32);
        self.regs[15] = target.wrapping_add(8); // 维持"regs[15] = 指令地址+8"约定
        self.pc_written = true;
    }

    /// BX：跳转并按目标地址位 0 切换 ARM/Thumb 状态。
    fn branch_exchange(&mut self, insn: u32) {
        let target = self.regs[(insn & 0xF) as usize];
        let to_thumb = target & 1 != 0;
        self.cpsr = if to_thumb {
            self.cpsr | (1 << 5)
        } else {
            self.cpsr & !(1 << 5)
        };
        self.regs[15] = if to_thumb {
            (target & !1).wrapping_add(4)
        } else {
            (target & !3).wrapping_add(8)
        };
        self.pc_written = true;
    }

    /// LDR/STR：字/字节 × 前/后索引 × 立即数/寄存器偏移。
    /// 半字与有符号变体在位 4、7 同 1 的另一编码空间（M1-b）。
    fn single_transfer(&mut self, insn: u32, bus: &mut Bus) -> Result<(), CpuError> {
        let pre_index = insn & (1 << 24) != 0;
        let up = insn & (1 << 23) != 0;
        let byte = insn & (1 << 22) != 0;
        let writeback = insn & (1 << 21) != 0;
        let load = insn & (1 << 20) != 0;
        let rn = ((insn >> 16) & 0xF) as usize;
        let rd = ((insn >> 12) & 0xF) as usize;

        let base = self.regs[rn]; // Rn=PC 时 = 指令地址 + 8（架构规定）
        let offset = if insn & (1 << 25) != 0 {
            // 寄存器偏移，可带立即数移位（位 4 必为 0）
            let rm = (insn & 0xF) as usize;
            let ty = alu::ShiftType::from((insn >> 5) & 3);
            alu::shift_imm(self.regs[rm], ty, (insn >> 7) & 0x1F, false).0
        } else {
            insn & 0xFFF
        };
        let offset = if up { offset } else { offset.wrapping_neg() };

        let addr = if pre_index {
            base.wrapping_add(offset)
        } else {
            base
        };

        if load {
            let data = if byte {
                bus.read_u8(addr) as u32
            } else {
                // 非对齐字读取的真机怪癖：按 (addr&3)*8 位循环右移
                bus.read_u32(addr).rotate_right((addr & 3) * 8)
            };
            if rd == 15 {
                self.write_pc_arm(data, false); // LDR PC：跳转（v4 无状态切换）
            } else {
                self.regs[rd] = data;
            }
        } else {
            // STR 的 Rd=PC：存的是"指令地址 + 12"，比读 PC 的可见值还多 4
            // ——架构为对齐内存访问流水线规定的怪癖（GBATEK）
            let data = self.regs[rd].wrapping_add(if rd == 15 { 4 } else { 0 });
            if byte {
                bus.write_u8(addr, data as u8);
            } else {
                bus.write_u32(addr, data);
            }
        }

        // 基址写回：后索引恒写回；前索引仅 W=1。Rn=PC 时写回无意义，跳过。
        if (!pre_index || writeback) && rn != 15 {
            self.regs[rn] = base.wrapping_add(offset);
        }
        Ok(())
    }

    /// 乘法族：MUL/MLA（32 位结果）与 UMULL/UMLAL/SMULL/SMLAL（64 位结果）。
    /// 操作数字段顺序已对照 mGBA `decoder-arm.c` 的 DEFINE_MULTIPLY_DECODER
    /// 宏确认：短乘 Rd=位19-16、Rn=位15-12、Rs=位11-8、Rm=位3-0。
    fn multiply(&mut self, insn: u32) {
        let s = insn & (1 << 20) != 0;
        let accumulate = insn & (1 << 21) != 0;
        let rm = (insn & 0xF) as usize;
        let rs = ((insn >> 8) & 0xF) as usize;
        if insn & (1 << 23) == 0 {
            // 短乘法：Rd = Rm × Rs (+ Rn)
            let rd = ((insn >> 16) & 0xF) as usize;
            let rn = ((insn >> 12) & 0xF) as usize;
            let mut result = self.regs[rm].wrapping_mul(self.regs[rs]);
            if accumulate {
                result = result.wrapping_add(self.regs[rn]);
            }
            if s {
                self.set_nz(result); // ARM7 上短乘法的 C/V 不可预测，保持不动
            }
            self.regs[rd] = result;
        } else {
            // 长乘法：RdHi:RdLo（64 位）= Rm × Rs (+ 原 RdHi:RdLo)
            let unsigned = insn & (1 << 22) != 0;
            let rd_hi = ((insn >> 16) & 0xF) as usize;
            let rd_lo = ((insn >> 12) & 0xF) as usize;
            let (a, b) = (self.regs[rm], self.regs[rs]);
            let mut product: u64 = if unsigned {
                u64::from(a) * u64::from(b)
            } else {
                ((a as i32) as i64).wrapping_mul((b as i32) as i64) as u64
            };
            if accumulate {
                let acc = (u64::from(self.regs[rd_hi]) << 32) | u64::from(self.regs[rd_lo]);
                product = product.wrapping_add(acc);
            }
            if s {
                self.set_flag_n(product >> 63 == 1);
                self.set_flag_z(product == 0);
            }
            self.regs[rd_hi] = (product >> 32) as u32;
            self.regs[rd_lo] = product as u32;
        }
    }

    /// SWI：软件中断。M1-d 将在此叠加 HLE——游戏的 BIOS 调用
    /// （LZ77 解压、除法等）将被直接模拟而不是跳进零 BIOS。
    fn swi(&mut self) {
        self.enter_exception(VECTOR_SWI, Mode::Supervisor);
    }
}

/// MSR 的 field 掩码：位 19-16 每 1 位控制 PSR 的一个字节
/// （0x1=控制域[T/模式/I/F]、0x2=扩展域、0x4=状态域、0x8=标志域[NZCV]）。
fn psr_field_mask(field: u32) -> u32 {
    let mut m = 0;
    if field & 0x1 != 0 {
        m |= 0x0000_00FF;
    }
    if field & 0x2 != 0 {
        m |= 0x0000_FF00;
    }
    if field & 0x4 != 0 {
        m |= 0x00FF_0000;
    }
    if field & 0x8 != 0 {
        m |= 0xFF00_0000;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    const IWRAM: u32 = 0x0300_0000;

    /// 最小环境：0x100 字节空 ROM + 无 BIOS 启动的 CPU。
    fn setup() -> (Cpu, Bus) {
        (
            Cpu::new_no_bios(),
            Bus::new(Cartridge::from_bytes(vec![0; 0x100].into()).unwrap()),
        )
    }

    /// 构造一个"即将执行 addr 处指令"的 CPU（流水线约定：regs[15] = addr + 8）。
    fn setup_at(addr: u32) -> (Cpu, Bus) {
        let (mut cpu, bus) = setup();
        cpu.regs[15] = addr.wrapping_add(8);
        (cpu, bus)
    }

    #[test]
    fn mov_imm() {
        let (mut cpu, mut bus) = setup();
        cpu.arm_step(0xE3A0_00FF, &mut bus).unwrap(); // MOV r0, #0xFF
        assert_eq!(cpu.reg(0), 0xFF);
        assert_eq!(cpu.cpsr(), 0x1F); // 无 S：标志位不动
    }

    #[test]
    fn movs_sets_carry_from_shifter() {
        let (mut cpu, mut bus) = setup();
        cpu.arm_step(0xE3A0_0480, &mut bus).unwrap(); // MOV r0, #0x80000000（rot=8）
        cpu.arm_step(0xE1B0_1080, &mut bus).unwrap(); // MOVS r1, r0, LSL #1
        assert_eq!(cpu.reg(1), 0);
        assert!(cpu.cpsr() & (1 << 30) != 0, "结果为 0 → Z");
        assert!(cpu.cpsr() & (1 << 29) != 0, "移出原 bit31 → C");
        assert!(cpu.cpsr() & (1 << 28) == 0, "逻辑操作不动 V");
    }

    #[test]
    fn adds_sets_c_on_unsigned_overflow() {
        let (mut cpu, mut bus) = setup();
        cpu.arm_step(0xE3E0_0000, &mut bus).unwrap(); // MVN r0, #0 → 0xFFFFFFFF
        cpu.arm_step(0xE290_0002, &mut bus).unwrap(); // ADDS r0, r0, #2
        assert_eq!(cpu.reg(0), 1);
        assert!(cpu.cpsr() & (1 << 29) != 0, "加法回绕 → C");
        assert!(cpu.cpsr() & (1 << 28) == 0, "负 + 正不溢出 → V=0");
        assert!(cpu.cpsr() & (1 << 30) == 0, "结果为 1 → Z=0");
    }

    #[test]
    fn cmp_does_not_write_destination() {
        let (mut cpu, mut bus) = setup();
        cpu.arm_step(0xE3A0_0005, &mut bus).unwrap(); // MOV r0, #5
        cpu.arm_step(0xE350_0005, &mut bus).unwrap(); // CMP r0, #5
        assert_eq!(cpu.reg(0), 5);
        assert!(cpu.cpsr() & (1 << 30) != 0, "相等 → Z");
        assert!(cpu.cpsr() & (1 << 29) != 0, "无借位 → C");
    }

    #[test]
    fn conditional_execution_skips() {
        let (mut cpu, mut bus) = setup();
        cpu.arm_step(0x13A0_0001, &mut bus).unwrap(); // MOVNE r0, #1（Z=0 → 执行）
        assert_eq!(cpu.reg(0), 1);
        cpu.arm_step(0xE3B0_5000, &mut bus).unwrap(); // MOVS r5, #0 → Z=1（别动 r0）
        cpu.arm_step(0x13A0_0002, &mut bus).unwrap(); // MOVNE r0, #2（Z=1 → 跳过）
        assert_eq!(cpu.reg(0), 1);
    }

    #[test]
    fn branch_and_link_sets_lr() {
        let (mut cpu, mut bus) = setup_at(IWRAM);
        cpu.arm_step(0xEB00_0010, &mut bus).unwrap(); // BL .+0x40
        assert_eq!(cpu.reg(14), IWRAM + 4); // LR = 本指令 + 4
        assert_eq!(cpu.reg(15), IWRAM + 0x50); // 目标 0x48 + 流水线 8
    }

    #[test]
    fn bx_switches_to_thumb() {
        let (mut cpu, mut bus) = setup();
        cpu.regs[3] = 0x0300_0101; // 位 0 = 1 → Thumb
        cpu.arm_step(0xE12F_FF13, &mut bus).unwrap(); // BX r3
        assert!(cpu.t());
        assert_eq!(cpu.reg(15), 0x0300_0104); // Thumb 目标 + 4
    }

    #[test]
    fn ldr_str_roundtrip_and_unaligned_rotation() {
        let (mut cpu, mut bus) = setup();
        cpu.regs[1] = 0x0200_0100;
        cpu.regs[2] = 0x1234_5678;
        cpu.arm_step(0xE581_2000, &mut bus).unwrap(); // STR r2, [r1]
        assert_eq!(bus.read_u32(0x0200_0100), 0x1234_5678);
        cpu.arm_step(0xE591_3000, &mut bus).unwrap(); // LDR r3, [r1]
        assert_eq!(cpu.reg(3), 0x1234_5678);
        // 非对齐 LDR：地址 +2 → 读出后按 16 位循环右移（真机怪癖）
        cpu.regs[1] = 0x0200_0102;
        cpu.arm_step(0xE591_4000, &mut bus).unwrap(); // LDR r4, [r1]
        assert_eq!(cpu.reg(4), 0x5678_1234);
    }

    #[test]
    fn post_index_writeback() {
        let (mut cpu, mut bus) = setup();
        cpu.regs[1] = 0x0200_0000;
        cpu.arm_step(0xE481_1004, &mut bus).unwrap(); // STR r1, [r1], #4
        assert_eq!(bus.read_u32(0x0200_0000), 0x0200_0000);
        assert_eq!(cpu.reg(1), 0x0200_0004); // 基址写回
    }

    #[test]
    fn ldr_pc_jumps() {
        let (mut cpu, mut bus) = setup();
        bus.write_u32(0x0200_0080, 0x0300_0200);
        cpu.regs[1] = 0x0200_0080;
        cpu.arm_step(0xE591_F000, &mut bus).unwrap(); // LDR pc, [r1]
        assert_eq!(cpu.reg(15), 0x0300_0208); // 目标 + 8
        assert!(!cpu.t());
    }

    #[test]
    fn msr_mrs_and_mode_switch() {
        let (mut cpu, mut bus) = setup();
        cpu.arm_step(0xE32F_F0D3, &mut bus).unwrap(); // MSR CPSR_all, #0xD3
        assert_eq!(cpu.mode(), Mode::Supervisor);
        assert_eq!(cpu.reg(13), 0x0300_7FE0); // svc 栈指针生效
        cpu.arm_step(0xE10F_1000, &mut bus).unwrap(); // MRS r1, CPSR
        assert_eq!(cpu.reg(1), 0x0000_00D3);
        cpu.arm_step(0xE32F_F01F, &mut bus).unwrap(); // MSR CPSR_all, #0x1F
        assert_eq!(cpu.mode(), Mode::System);
        assert_eq!(cpu.reg(13), 0x0300_7F00); // sys 栈指针恢复
    }

    #[test]
    fn swi_enters_supervisor() {
        let addr = IWRAM + 0x40;
        let (mut cpu, mut bus) = setup_at(addr);
        cpu.arm_step(0xEF00_0018, &mut bus).unwrap(); // SWI 0x18
        assert_eq!(cpu.mode(), Mode::Supervisor);
        assert_eq!(cpu.reg(14), addr + 4); // 返回地址 = 本指令 + 4
        assert_eq!(cpu.reg(15), VECTOR_SWI + 8);
        assert!(cpu.cpsr() & (1 << 7) != 0, "进入异常屏蔽 IRQ");
        assert_eq!(cpu.spsr[Mode::Supervisor.bank()], 0x1F, "SPSR 保存旧 CPSR");
    }

    #[test]
    fn movs_pc_restores_cpsr_from_spsr() {
        let addr = IWRAM;
        let (mut cpu, mut bus) = setup_at(addr);
        cpu.arm_step(0xEF00_0000, &mut bus).unwrap(); // SWI 0 → 进 SVC
        cpu.arm_step(0xE1B0_F00E, &mut bus).unwrap(); // MOVS pc, lr（异常返回）
        assert_eq!(cpu.cpsr(), 0x1F); // CPSR 从 SPSR 整体恢复
        assert_eq!(cpu.reg(15), addr + 12); // 回到 SWI 的下一条（+8 约定）
    }

    #[test]
    fn multiply_mla_and_umull() {
        let (mut cpu, mut bus) = setup();
        cpu.regs[1] = 3;
        cpu.regs[2] = 4;
        cpu.regs[3] = 5;
        // MLA r0, r1, r2, r3 → 3×4+5 = 17
        cpu.arm_step(0xE020_3291, &mut bus).unwrap();
        assert_eq!(cpu.reg(0), 17);
        cpu.regs[1] = 0xFFFF_FFFF;
        cpu.regs[2] = 2;
        // UMULL r4(lo), r5(hi), r1, r2 → 0x1_FFFF_FFFE
        cpu.arm_step(0xE0C5_4291, &mut bus).unwrap(); // UMULL r4(lo), r5(hi), r1, r2
        assert_eq!(cpu.reg(4), 0xFFFF_FFFE);
        assert_eq!(cpu.reg(5), 1);
    }

    #[test]
    fn unimplemented_instructions_report_error() {
        let (mut cpu, mut bus) = setup();
        // LDMIA r0, {r1}（位 27-25 = 100）：M1-b 实现
        let err = cpu.arm_step(0xE890_0002, &mut bus).unwrap_err();
        assert!(matches!(err, CpuError::Unimplemented { .. }));
    }
}
