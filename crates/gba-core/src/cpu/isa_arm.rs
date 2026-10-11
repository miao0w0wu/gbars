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
            0b000 => self.category_000(insn, bus),
            0b001 => self.data_processing(insn), // 立即数版（含 MSR 立即数）
            0b010 => self.single_transfer(insn, bus),
            0b011 | 0b110 => {
                // 协处理器空间：GBA 没有游戏可用的协处理器，真机行为
                // 就是未定义指令异常
                self.enter_exception(VECTOR_UNDEFINED, Mode::Undefined);
                Ok(())
            }
            0b100 => {
                self.block_transfer(insn, bus);
                Ok(())
            }
            0b101 => {
                self.branch(insn);
                Ok(())
            }
            _ => {
                if insn & (1 << 24) != 0 {
                    self.swi(bus, insn & 0x00FF_FFFF); // 1111 + 位24=1：SWI
                } else {
                    self.enter_exception(VECTOR_UNDEFINED, Mode::Undefined); // CDP/MCR
                }
                Ok(())
            }
        }
    }

    /// 位 27–25 = 000 的杂烩区：数据处理、BX、乘法、SWP、半字传输。
    fn category_000(&mut self, insn: u32, bus: &mut Bus) -> Result<(), CpuError> {
        // BX 占据 cond 0001 0010 1111 1111 0001 xxxx 的编码空间
        if insn & 0x0FFF_FFF0 == 0x012F_FF10 {
            self.branch_exchange(insn);
            return Ok(());
        }
        // 位 7、4 同时为 1 → 乘法 / SWP / 半字传输 的判别区
        if insn & 0x90 == 0x90 {
            if insn & 0x60 == 0 {
                if insn & (1 << 24) == 0 {
                    self.multiply(insn);
                } else {
                    self.swap(insn, bus);
                }
            } else {
                self.half_transfer(insn, bus);
            }
            return Ok(());
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
        // 写目标 PC：ARM 态 = 目标 + 8；若 SPSR 恢复带回 T=1 则进 Thumb
        // （ARMv4 没有 BLX，靠异常返回把状态切到 Thumb）
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

    /// SWI：软件中断。无 BIOS 模式下走 HLE——`bios::hle_swi` 直接算出
    /// BIOS 调用的结果（改寄存器/内存），PC 像普通指令一样推进；只有
    /// HLE 未覆盖的编号才回落到异常入口。
    fn swi(&mut self, bus: &mut Bus, comment: u32) {
        crate::bios::hle_swi(self, bus, comment);
    }

    /// LDM/STM：一次搬运多个寄存器。函数序言/尾声（压栈/出栈）全靠它。
    ///
    /// 四种寻址模式由 P 位（前/后）与 U 位（增/减）组合而成，用 R13 当
    /// 基址时习惯写成：
    ///
    /// | 模式 | 编码 | 含义 | 别名 |
    /// |------|------|------|------|
    /// | IA | P=0 U=1 | 从基址起，先访问后自增 | `STMIA` / `LDMFD`（满递减栈出栈） |
    /// | IB | P=1 U=1 | 基址先增，再访问 | `STMIB` / `LDMED` |
    /// | DA | P=0 U=0 | 从基址起，先访问后自减 | `STMDA` / `LDMFA` |
    /// | DB | P=1 U=0 | 基址先减，再访问 | `STMDB` / `LDMFD` 入栈（`PUSH`） |
    ///
    /// 统一算法：按内存地址从低到高逐个处理寄存器；方向 U=0 时**最高编号
    /// 寄存器落在最低地址**（这决定多个寄存器入栈后的内存排布）。
    /// 地址计算与此处顺序均已对照 mGBA `GBALoadMultiple/StoreMultiple`
    /// （`src/gba/memory.c`）。
    fn block_transfer(&mut self, insn: u32, bus: &mut Bus) {
        let pre = insn & (1 << 24) != 0;
        let up = insn & (1 << 23) != 0;
        let s_bit = insn & (1 << 22) != 0;
        let writeback = insn & (1 << 21) != 0;
        let load = insn & (1 << 20) != 0;
        let rn = ((insn >> 16) & 0xF) as usize;
        let list = insn & 0xFFFF;
        let count = list.count_ones();
        let base = self.regs[rn];

        // 计算"最低地址"与"寄存器编号 → 地址"的映射。
        //
        // 统一算法（GBATEK "LSM"）：先求本次传输覆盖的最低地址 low，然后
        //   - 递增（U=1）：编号小的寄存器在低地址：addr(r) = low + k*4
        //   - 递减（U=0）：编号大的寄存器在低地址：addr(r) = low + (count-1-k)*4
        // 其中 k 是 r 在列表中的序号（从 0 起）。四种模式的 low 与结束地址：
        //
        // | 模式 | low | 结束后基址 |
        // |------|-----|-----------|
        // | IA (P0 U1) | base | base + count*4 |
        // | IB (P1 U1) | base + 4 | base + count*4 |
        // | DA (P0 U0) | base - count*4 + 4 | base - count*4 + 4 |
        // | DB (P1 U0) | base - count*4 | base - count*4 |
        let low = match (pre, up) {
            (false, true) => base,                                          // IA
            (true, true) => base.wrapping_add(4),                           // IB
            (false, false) => base.wrapping_sub(4 * count).wrapping_add(4), // DA
            (true, false) => base.wrapping_sub(4 * count),                  // DB
        };
        // 基址写回后的值：递增模式落在最高地址之上，递减模式落在 low
        let new_base = if up {
            base.wrapping_add(4 * count)
        } else {
            low
        };

        // S 位 + 非 User 模式：LDM 装载 PC（R15 在列表里）时恢复 CPSR；
        // 否则在 User 态银行里访问寄存器（用户态块传输，很少见）
        let restore_spsr = load && s_bit && list & 0x8000 != 0 && self.mode().has_spsr();
        let mut regs = 0u32;
        for r in 0..16u32 {
            if list & (1 << r) == 0 {
                continue;
            }
            // 无论递增还是递减，**编号小的寄存器总在低地址**——
            // "递减"描述的是基址如何算出 low，不是寄存器到地址的映射方向。
            // （这正是 STMDB/PUSH 后内存里 r0 在最低地址、rN 在最高地址的原因）
            let a = low.wrapping_add(regs * 4);
            regs += 1;
            if load {
                let value = bus.read_u32(a);
                if r == 15 {
                    // LDM 装 PC：PC 变化，且可能恢复 CPSR（异常返回）
                    self.write_pc_arm(value, restore_spsr);
                } else {
                    self.regs[r as usize] = value;
                }
            } else {
                let value = self.regs[r as usize];
                bus.write_u32(a, value);
            }
        }

        // 基址写回。关键规则（GBATEK "LSM" 的 ULD/W 位说明）：
        // 若基址寄存器 Rn 也在传输列表中，写回值要分情况——
        //   - 装载（LDM）：Rn 是列表中**最低编号**时，写回被装载的值
        //     （跳过新基址）；否则写回新基址；
        //   - 存储（STM）：Rn 是列表中**最低编号**时，写回新基址；
        //     否则写回被存进去的旧值（即不写回）。
        // 直观理解：硬件先算好新基址，只有当"内存访问用的旧值"被 Rn 自己
        // 覆盖时才让新基址生效，避免压栈时把栈指针覆盖成待存的数据。
        if writeback {
            let rn_in_list = list & (1 << rn) != 0;
            let rn_is_lowest = rn_in_list && (list & ((1u32 << rn) - 1)) == 0;
            let do_writeback = if !rn_in_list {
                true
            } else if load {
                // LDM：Rn 是最低编号 → 已被装载覆盖，不再写回
                !rn_is_lowest
            } else {
                // STM：Rn 是最低编号 → 写回新基址
                rn_is_lowest
            };
            if do_writeback {
                self.regs[rn] = new_base;
            }
        }
    }

    /// SWP/SWPB：原子交换（读旧值 → 写新值 → 目标寄存器得到旧值）。
    /// 没有真正的"原子性"问题——模拟器是单线程顺序执行。
    fn swap(&mut self, insn: u32, bus: &mut Bus) {
        let byte = insn & (1 << 22) != 0;
        let rn = ((insn >> 16) & 0xF) as usize; // 地址基址
        let rd = ((insn >> 12) & 0xF) as usize; // 目标（接收旧值）
        let rm = (insn & 0xF) as usize; // 源（要写入的新值）
        let addr = self.regs[rn];
        let loaded = if byte {
            let old = bus.read_u8(addr);
            bus.write_u8(addr, self.regs[rm] as u8);
            u32::from(old)
        } else {
            let old = bus.read_u32(addr);
            bus.write_u32(addr, self.regs[rm]);
            old
        };
        self.regs[rd] = loaded;
    }

    /// 半字与有符号字节/半字传输（LDRH/STRH/LDRSB/LDRSH）。
    ///
    /// 编码空间与 SWP 共享"位 7、4 = 1"区，用位 6-5 区分：
    /// `01 = H`、`10 = SB`、`11 = SH`。SH 与 SB 装载结果要**符号扩展**
    /// 到 32 位，这是与 LDRB（零扩展）的本质区别。
    fn half_transfer(&mut self, insn: u32, bus: &mut Bus) {
        let pre = insn & (1 << 24) != 0;
        let up = insn & (1 << 23) != 0;
        let immediate = insn & (1 << 22) != 0;
        let writeback = insn & (1 << 21) != 0;
        let load = insn & (1 << 20) != 0;
        let rn = ((insn >> 16) & 0xF) as usize;
        let rd = ((insn >> 12) & 0xF) as usize;
        let kind = (insn >> 5) & 0x3; // 01=H, 10=SB, 11=SH

        let offset = if immediate {
            ((insn >> 4) & 0xF0) | (insn & 0xF) // 高 4 位与低 4 位拼成 8 位偏移
        } else {
            self.regs[(insn & 0xF) as usize]
        };
        let offset = if up { offset } else { offset.wrapping_neg() };
        let base = self.regs[rn];
        let addr = if pre { base.wrapping_add(offset) } else { base };

        if load {
            let value = match kind {
                0x1 => u32::from(bus.read_u16(addr)), // LDRH：零扩展到 32 位
                0x2 => alu::sign_extend(u32::from(bus.read_u8(addr)), 8) as u32, // LDRSB
                _ => alu::sign_extend(u32::from(bus.read_u16(addr)), 16) as u32, // LDRSH
            };
            if rd == 15 {
                // LDRH/LDRSB/LDRSH 目标为 PC：ARMv4 未定义（M5 架构才允许）。
                // 真机行为不可预测，这里按 LDR PC 跳转处理以保证不停摆。
                self.write_pc_arm(value, false);
            } else {
                self.regs[rd] = value;
            }
        } else {
            match kind {
                0x1 => bus.write_u16(addr, self.regs[rd] as u16),
                _ => bus.write_u16(addr, self.regs[rd] as u16), // STRH（SB/SH 无存储形式）
            }
        }

        if (!pre || writeback) && rn != 15 {
            self.regs[rn] = base.wrapping_add(offset);
        }
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
    fn ldm_stm_with_writeback_and_pc() {
        let (mut cpu, mut bus) = setup();
        cpu.regs[0] = 0x0200_0000;
        cpu.regs[1] = 0x1111_1111;
        cpu.regs[2] = 0x2222_2222;
        // STMIA r0!, {r1, r2}：先存后自增，基址写回
        cpu.arm_step(0xE8A0_0006, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x0200_0000), 0x1111_1111);
        assert_eq!(bus.read_u32(0x0200_0004), 0x2222_2222);
        assert_eq!(cpu.reg(0), 0x0200_0008);
        // LDMIA r0, {r3, r4}：从指定地址重新装载
        cpu.regs[0] = 0x0200_0000;
        cpu.arm_step(0xE890_0018, &mut bus).unwrap();
        assert_eq!(cpu.reg(3), 0x1111_1111);
        assert_eq!(cpu.reg(4), 0x2222_2222);
    }

    #[test]
    fn stmdb_is_full_descending_push() {
        // PUSH {r0-r2} 的 ARM 等价写法：STMDB sp!, {r0-r2}
        // 寄存器按下标从高到低入栈，编号小的落在低地址
        let (mut cpu, mut bus) = setup();
        cpu.regs[13] = 0x0300_7F00;
        cpu.regs[0] = 0xAA;
        cpu.regs[1] = 0xBB;
        cpu.regs[2] = 0xCC;
        cpu.arm_step(0xE92D_0007, &mut bus).unwrap(); // STMDB sp!, {r0,r1,r2}
        assert_eq!(cpu.reg(13), 0x0300_7EF4); // SP 下移 12
        assert_eq!(bus.read_u32(0x0300_7EF4), 0xAA); // r0 在最低地址
        assert_eq!(bus.read_u32(0x0300_7EFC), 0xCC);
    }

    #[test]
    fn ldm_pc_is_exception_return() {
        let (mut cpu, mut bus) = setup();
        let stub_addr = 0x0200_0020;
        bus.write_u32(stub_addr, 0x1111_1111);
        bus.write_u32(stub_addr + 4, 0x0300_0500);
        // 进入 SWI 后 LDMFD sp!, {r4, pc}（LDMEA 的异常返回写法）
        cpu.regs[13] = stub_addr;
        let _ = cpu.arm_step(0xEF00_0000, &mut bus); // SWI → svc 模式
        let svc_sp = cpu.reg(13); // svc 的栈（由银行提供）
        cpu.regs[13] = svc_sp;
        bus.write_u32(svc_sp, 0x1111_1111);
        bus.write_u32(svc_sp + 4, 0x0300_0500);
        // LDMIA sp!, {r4, pc}
        cpu.arm_step(0xE8BD_8010, &mut bus).unwrap();
        assert_eq!(cpu.reg(4), 0x1111_1111);
        assert_eq!(cpu.reg(15), 0x0300_0508); // 跳转：目标 + 8
    }

    #[test]
    fn swp_exchanges_and_swpb() {
        let (mut cpu, mut bus) = setup();
        bus.write_u32(0x0200_0010, 0xCAFE_BABE);
        cpu.regs[1] = 0x0200_0010;
        cpu.regs[2] = 0x1234_5678;
        cpu.arm_step(0xE101_2092, &mut bus).unwrap(); // SWP r2, r2, [r1]
        assert_eq!(cpu.reg(2), 0xCAFE_BABE); // 拿到旧值
        assert_eq!(bus.read_u32(0x0200_0010), 0x1234_5678); // 写进新值

        bus.write_u8(0x0200_0020, 0x11);
        cpu.regs[1] = 0x0200_0020;
        cpu.regs[3] = 0x99;
        cpu.arm_step(0xE141_3093, &mut bus).unwrap(); // SWPB r3, r3, [r1]
        assert_eq!(cpu.reg(3), 0x11);
        assert_eq!(bus.read_u8(0x0200_0020), 0x99);
    }

    #[test]
    fn half_and_signed_transfers() {
        let (mut cpu, mut bus) = setup();
        bus.write_u16(0x0200_0000, 0x8001);
        cpu.regs[1] = 0x0200_0000;
        cpu.regs[2] = 0x0200_0010;

        cpu.arm_step(0xE1D1_30B0, &mut bus).unwrap(); // LDRH r3, [r1]
        assert_eq!(cpu.reg(3), 0x8001); // 零扩展

        cpu.arm_step(0xE1D1_40D0, &mut bus).unwrap(); // LDRSB r4, [r1]
        assert_eq!(cpu.reg(4), 0x0000_0001); // 低字节 0x01 → 正数

        cpu.arm_step(0xE1D1_50F0, &mut bus).unwrap(); // LDRSH r5, [r1]
        assert_eq!(cpu.reg(5), 0xFFFF_8001); // 半字符号扩展到 32 位

        // STRH 立即数偏移写回
        cpu.regs[6] = 0x1234;
        cpu.arm_step(0xE1C2_60B2, &mut bus).unwrap(); // STRH r6, [r2, #2]
        assert_eq!(bus.read_u16(0x0200_0012), 0x1234);
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
    fn post_index_halfword_writeback_and_multiply_distinction() {
        let (mut cpu, mut bus) = setup();
        bus.write_u16(0x0200_0000, 0x5678);
        cpu.regs[1] = 0x0200_0000;
        cpu.regs[2] = 4;
        cpu.arm_step(0xE091_30B2, &mut bus).unwrap();
        assert_eq!(cpu.reg(3), 0x5678);
        assert_eq!(cpu.reg(1), 0x0200_0004);

        cpu.regs[1] = 0x0200_0010;
        cpu.regs[2] = 0xABCD;
        cpu.arm_step(0xE0C1_20B2, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x0200_0010), 0xABCD);
        assert_eq!(cpu.reg(1), 0x0200_0012);

        cpu.regs[2] = 0x0001_0000;
        cpu.regs[3] = 0x0002_0000;
        cpu.arm_step(0xE081_0392, &mut bus).unwrap();
        assert_eq!(cpu.reg(1), 2);
        assert_eq!(cpu.reg(0), 0);
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
    fn swi_div_is_hle() {
        // 无 BIOS 模式下 SWI 直接走 HLE：不切模式、不碰 PC，只有
        // BIOS 调用的副作用（这里用 Div 验证：r0=商、r1=余数、r3=|商|）
        let (mut cpu, mut bus) = setup();
        cpu.regs[0] = 47; // num
        cpu.regs[1] = 5; // denom
        cpu.arm_step(0xEF00_0006, &mut bus).unwrap(); // SWI 0x06 (Div)
        assert_eq!(cpu.reg(0), 9);
        assert_eq!(cpu.reg(1), 2);
        assert_eq!(cpu.reg(3), 9);
        // PC 像普通指令一样推进（HLE 对调用者透明）
        assert_eq!(cpu.reg(15), 0x0800_000C);
        assert_eq!(cpu.mode(), Mode::System); // 模式没变
    }

    #[test]
    fn movs_pc_restores_cpsr_from_spsr() {
        // 异常返回路径（MOVS pc, lr / LDM 带 S 位装 PC）依赖 SPSR 恢复。
        // SWI 已被 HLE 接管，这里手动构造异常入口来测。
        let addr = IWRAM;
        let (mut cpu, mut bus) = setup_at(addr);
        cpu.enter_exception(0x08, Mode::Supervisor); // 模拟 SWI 异常入口
        cpu.arm_step(0xE1B0_F00E, &mut bus).unwrap(); // MOVS pc, lr（异常返回）
        assert_eq!(cpu.cpsr(), 0x1F); // CPSR 从 SPSR 整体恢复
        assert_eq!(cpu.reg(15), addr + 12); // 回到"SWI 的下一条"（+8 约定）
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
    fn coprocessor_instructions_enter_undefined() {
        // 协处理器指令（位 27-25 = 011/110）在 GBA 上触发未定义指令异常
        // （Und 模式、跳向量 0x04）——这是真机行为，不是"未实现"。
        let (mut cpu, mut bus) = setup();
        cpu.arm_step(0xEC00_0000, &mut bus).unwrap(); // 协处理器 CDP 空间
        assert_eq!(cpu.mode(), Mode::Undefined);
        assert_eq!(cpu.reg(15), 0x04 + 8); // 向量 0x04 的"指令地址+8"
    }
}
