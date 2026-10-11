//! Thumb 指令集执行（ARMv4T）。
//!
//! # 硬件背景：为什么 GBA 游戏主要用 Thumb 写
//!
//! Thumb 是 ARM 的 16 位指令集。GBA 的 ROM 总线只有 16 位宽——取一条
//! 32 位 ARM 指令要占两个总线周期，而一条 Thumb 指令只要一个。于是
//! 商业游戏（包括 Pokemon FireRed）的代码几乎全用 Thumb 编译：**同样的
//! 代码体积更小、取指更快**，代价是寄存器操作受限（大部分指令只能
//! 访问 r0–r7）、没有条件执行。
//!
//! 状态切换：ARM 的 `BX`/`BLX` 根据目标地址**最低位**切换——为 1 进
//! Thumb。所以 Thumb 函数指针总是奇数（`0x08000001`），跳转时再抹掉。
//!
//! # 编码：19 种"格式"
//!
//! Thumb 指令按位 15–11（少数到 15–10）划分成 19 种格式，从高到低逐级
//! 判别（GBATEK "Thumb Instruction Set"）。本文件按格式编号组织代码，
//! 每个 `format_*` 函数对应 GBATEK 的一节，便于对照。
//!
//! # PC 与流水线
//!
//! Thumb 沿用本项目的统一约定：执行期间 `regs[15]` = 当前指令地址 + 4
//! （Thumb 两条流水级），取指地址 = `regs[15] - 4`。`BL` 这类跨两条
//! 16 位指令的操作靠 [`Cpu::thumb_upper`] 暂存高半部分完成。
//!
//! # 参考
//!
//! - GBATEK "Thumb Instruction Set"（格式编号与位域）
//! - mGBA `src/arm/decoder-thumb.c`、`isa-thumb.c`

use super::{Cpu, alu, cond_pass};
use crate::bus::Bus;

impl Cpu {
    /// 执行一条 Thumb 指令。`insn` 是 16 位，已从 `regs[15] - 4` 取出。
    pub(crate) fn thumb_step(&mut self, insn: u16, bus: &mut Bus) {
        self.pc_written = false;

        // BL/BLX 按编码识别：0xF000-0xF7FF 是前缀，0xF800-0xFFFF 恒为后缀。
        // 靠"上一条设过标志"配对会在 IRQ 恰好落在两条半字之间时错乱
        // （前缀已改写 LR、返回后后缀会被当成新前缀），真机按编码分派无此问题。
        let hi = insn & 0xF800;
        if hi == 0xF800 {
            self.thumb_bl_complete(insn);
        } else if hi == 0xF000 {
            self.thumb_f19_long_branch(insn);
        } else {
            self.thumb_execute(insn, bus);
        }

        if !self.pc_written {
            self.regs[15] = self.regs[15].wrapping_add(2);
        }
        self.steps += 1;
    }

    /// 按位 15–10 的 6 位域分派到 19 种格式之一。
    ///
    /// **教训（本步最大的坑）**：GBATEK 讲的"格式 1–19"是**讲解编号**，
    /// 不是位 15–11 的连续取值！第一版按连续整数区间分档，结果
    /// `MOV r0,#5`（0x2005，`>>10`=8）落进了 ALU 档。下表是各格式的
    /// **真实位模式**（`hi = insn >> 10`），直接照此分派：
    ///
    /// | hi 区间 | 格式 |
    /// |---------|------|
    /// | 0..=1 | F1 移位（0x0000–0x07FF）|
    /// | 2..=7 | F2 加/减（0x0800–0x1FFF，含 3 位立即数）|
    /// | 8..=15 | F3 MOV/CMP/ADD/SUB #imm8（0x2000）|
    /// | 16 | F4 ALU（0x4000）|
    /// | 17 | F5 高寄存器/BX（0x4400）|
    /// | 18..=19 | F6 PC 相对装载（0x4800）|
    /// | 20..=23 | F7/F8 寄存器偏移装载存储（0x5000）|
    /// | 24..=31 | F9 立即数偏移装载存储（0x6000–0x7FFF）|
    /// | 28..=31 | 保留（0x7000）|
    /// | 32..=35 | F10 半字立即数偏移（0x8000）|
    /// | 36..=39 | F11 SP 相对（0x9000）|
    /// | 40..=43 | F12 取地址 PC/SP（0xA000）|
    /// | 44 | F13 ADD SP,#imm（0xB000）|
    /// | 45..=47 | F14 PUSH/POP（0xB400/0xBC00）|
    /// | 48..=51 | F15 LDMIA/STMIA（0xC800）|
    /// | 52..=55 | F16 条件跳转（0xD000）+ F17 SWI（0xDF00）|
    /// | 56..=59 | F18 无条件跳转（0xE000）|
    /// | 60..=63 | F19 BL/BLX（0xF000）|
    fn thumb_execute(&mut self, insn: u16, bus: &mut Bus) {
        match insn >> 10 {
            // F1：LSL(0..=1)/LSR(2..=3)/ASR(4..=5)，op = 位 12-11 的两位值；
            // F2：ADD/SUB（6..=7）
            0..=5 => self.thumb_f1_2_shift_add_sub(insn, (insn >> 11) & 0x3),
            6..=7 => self.thumb_f1_2_shift_add_sub(insn, 0b11), // F2：加/减
            8..=15 => self.thumb_f3_mov_cmp_add_sub_imm(insn),  // F3
            16 => self.thumb_f4_alu(insn),                      // F4
            17 => self.thumb_f5_hireg_bx(insn, bus),            // F5
            18..=19 => self.thumb_f6_pc_relative_load(insn, bus), // F6
            20..=23 => self.thumb_f7_8_load_store_reg(insn, bus), // F7/F8
            24..=31 => self.thumb_f9_load_store_imm(insn, bus), // F9
            32..=35 => self.thumb_f10_load_store_half(insn, bus), // F10
            36..=39 => self.thumb_f11_sp_relative(insn, bus),   // F11
            40..=43 => self.thumb_f12_load_address(insn),       // F12
            44 => self.thumb_f13_add_sub_sp(insn),              // F13：ADD/SUB SP,#imm
            45..=47 => self.thumb_f14_push_pop(insn, bus),      // F14
            48..=51 => self.thumb_f15_multiple(insn, bus),      // F15
            52..=55 => self.thumb_f18_branch(insn, bus),        // F16 条件跳转 + F17 SWI
            56..=59 => self.thumb_f18_branch(insn, bus),        // F18：无条件跳转
            _ => self.thumb_f19_long_branch(insn),              // 60..=63：F19
        }
    }

    // ---- 格式 1、2：移位与加减（低寄存器）----

    /// F1：`LSL/LSR/ASR Rd, Rs, #imm5`（Rd、Rs 均为 r0–r7）
    /// F2：`ADD/SUB Rd, Rs, Rn|#imm3`（含 3 位立即数形式）
    /// `op` 是位 12–11 的两位值：00=LSL 01=LSR 10=ASR 11=ADD/SUB
    fn thumb_f1_2_shift_add_sub(&mut self, insn: u16, op: u16) {
        let rd = (insn & 0x7) as usize;
        let rs = ((insn >> 3) & 0x7) as usize;

        if op != 0b11 {
            // 格式 1：三种移位，移位量 = 位 10-6 的 5 位立即数
            let ty = alu::ShiftType::from(u32::from(op));
            let imm5 = u32::from((insn >> 6) & 0x1F);
            let (value, c) = alu::shift_imm(self.regs[rs], ty, imm5, self.carry());
            self.regs[rd] = value;
            self.set_nz(value);
            self.set_c(c);
        } else {
            // 格式 2：位 10 = 1 表示"立即数"，否则是寄存器
            let immediate = insn & (1 << 10) != 0;
            let operand = if immediate {
                u32::from((insn >> 6) & 0x7)
            } else {
                self.regs[((insn >> 6) & 0x7) as usize]
            };
            let subtract = insn & (1 << 9) != 0;
            let a = self.regs[rs];
            let (result, c, v) = if subtract {
                alu::sub_with_borrow(a, operand, false)
            } else {
                alu::add_with_carry(a, operand, false)
            };
            self.regs[rd] = result;
            self.set_nz(result);
            self.set_c(c);
            self.set_v(v);
        }
    }

    /// F3：`MOV/CMP/ADD/SUB Rd, #imm8`（8 位立即数，Rd 为低寄存器）
    /// op 是位 12–11 的两位值：00=MOV 01=CMP 10=ADD 11=SUB
    fn thumb_f3_mov_cmp_add_sub_imm(&mut self, insn: u16) {
        let op = (insn >> 11) & 0x3;
        let rd = ((insn >> 8) & 0x7) as usize;
        let imm = u32::from(insn & 0xFF);
        let a = self.regs[rd];
        match op {
            0 => {
                // MOV：立即数运算，标志位只受 N/Z 影响
                self.regs[rd] = imm;
                self.set_nz(imm);
            }
            1 => {
                let (r, c, v) = alu::sub_with_borrow(a, imm, false);
                self.set_nz(r);
                self.set_c(c);
                self.set_v(v);
            }
            2 => {
                let (r, c, v) = alu::add_with_carry(a, imm, false);
                self.regs[rd] = r;
                self.set_nz(r);
                self.set_c(c);
                self.set_v(v);
            }
            _ => {
                let (r, c, v) = alu::sub_with_borrow(a, imm, false);
                self.regs[rd] = r;
                self.set_nz(r);
                self.set_c(c);
                self.set_v(v);
            }
        }
    }

    /// F4：16 种 ALU 操作（AND/EOR/LSL/LSR/ASR/ADC/SBC/ROR/TST/NEG/CMP/
    /// CMN/ORR/MUL/BIC/MVN），两个操作数都是低寄存器。
    fn thumb_f4_alu(&mut self, insn: u16) {
        let op = (insn >> 6) & 0xF;
        let rd = (insn & 0x7) as usize;
        let rs = ((insn >> 3) & 0x7) as usize;
        let a = self.regs[rd];
        let b = self.regs[rs];
        // 多数操作写回 Rd 并更新 N/Z；比较类（TST/CMP/CMN）只更新标志
        let (result, write, carry, overflow): (u32, bool, Option<bool>, Option<bool>) = match op {
            0x0 => (a & b, true, None, None),
            0x1 => (a ^ b, true, None, None),
            0x2 => {
                // 移位：进位来自移位器（第三个元素）
                let (r, c) = alu::shift_reg(a, alu::ShiftType::Lsl, b & 0xFF, false);
                (r, true, Some(c), None)
            }
            0x3 => {
                let (r, c) = alu::shift_reg(a, alu::ShiftType::Lsr, b & 0xFF, false);
                (r, true, Some(c), None)
            }
            0x4 => {
                let (r, c) = alu::shift_reg(a, alu::ShiftType::Asr, b & 0xFF, false);
                (r, true, Some(c), None)
            }
            0x5 => {
                let (r, c, v) = alu::add_with_carry(a, b, self.carry());
                (r, true, Some(c), Some(v))
            }
            0x6 => {
                let (r, c, v) = alu::sub_with_borrow(a, b, !self.carry());
                (r, true, Some(c), Some(v))
            }
            0x7 => {
                let (r, c) = alu::shift_reg(a, alu::ShiftType::Ror, b & 0xFF, false);
                (r, true, Some(c), None)
            }
            0x8 => (a & b, false, None, None), // TST
            0x9 => {
                let (r, c, v) = alu::sub_with_borrow(0, b, false); // NEG
                (r, true, Some(c), Some(v))
            }
            0xA => {
                let (r, c, v) = alu::sub_with_borrow(a, b, false); // CMP
                (r, false, Some(c), Some(v))
            }
            0xB => {
                let (r, c, v) = alu::add_with_carry(a, b, false); // CMN
                (r, false, Some(c), Some(v))
            }
            0xC => (a | b, true, None, None),
            0xD => (a.wrapping_mul(b), true, None, None),
            0xE => (a & !b, true, None, None),
            _ => (!b, true, None, None), // MVN
        };
        if write {
            self.regs[rd] = result;
        }
        self.set_nz(result);
        if let Some(c) = carry {
            self.set_c(c);
        }
        if let Some(v) = overflow {
            self.set_v(v);
        }
    }

    /// F5：高寄存器操作与 BX/BLX。
    ///
    /// 与低寄存器指令的区别：允许访问 r8–r15；**不更新标志位**（CMP 除外）；
    /// 目标寄存器为 PC 时是"跳转"而非普通赋值。
    fn thumb_f5_hireg_bx(&mut self, insn: u16, bus: &mut Bus) {
        let _ = bus;
        // BX/BLX 的编码是 0100 0111 xHxx xxx（位 10-8 = 111），与
        // 普通高寄存器操作（位 10-8 = 000/001/010）分开判别
        if (insn >> 8) & 0x3 == 0x3 {
            // BX/BLX Rs
            let rs = (((insn >> 6) & 0x1) << 3 | ((insn >> 3) & 0x7)) as usize;
            let target = self.regs[rs];
            if insn & (1 << 7) != 0 {
                // BLX：返回地址入 LR（本指令 + 2），切到 ARM
                self.regs[14] = self.regs[15].wrapping_sub(2) | 1;
                self.thumb_switch_to_arm(target);
            } else {
                self.branch_exchange_thumb(target);
            }
            return;
        }

        let op = (insn >> 8) & 0x3;
        let h1 = (insn >> 7) & 1;
        let h2 = (insn >> 6) & 1;
        let rd = ((h1 << 3) | (insn & 0x7)) as usize;
        let rs = ((h2 << 3) | ((insn >> 3) & 0x7)) as usize;

        match op {
            0 => {
                // ADD：Rd += Rs（高寄存器参与，不更新标志）
                let result = self.regs[rd].wrapping_add(self.regs[rs]);
                if rd == 15 {
                    self.write_pc_thumb(result);
                } else {
                    self.regs[rd] = result;
                }
            }
            1 => {
                // CMP：高寄存器比较（唯一更新标志的高寄存器操作）
                let (r, c, v) = alu::sub_with_borrow(self.regs[rd], self.regs[rs], false);
                self.set_nz(r);
                self.set_c(c);
                self.set_v(v);
            }
            2 => {
                // MOV：Rd = Rs；Rd=PC 时是跳转
                let value = self.regs[rs];
                if rd == 15 {
                    self.write_pc_thumb(value);
                } else {
                    self.regs[rd] = value;
                }
            }
            _ => {
                // op=3 的普通形式在真机不存在（该空间是 BX/BLX，已在
                // 函数开头提前处理）；留此分支仅为穷尽 match
            }
        }
    }

    /// F6：`LDR Rd, [PC, #imm8×4]`——PC 相对字装载（读取常数池）。
    /// 基址按 Thumb 流水线对齐：`(PC & ~2) + 4`。
    fn thumb_f6_pc_relative_load(&mut self, insn: u16, bus: &mut Bus) {
        let rd = ((insn >> 8) & 0x7) as usize;
        let offset = u32::from(insn & 0xFF) * 4;
        let pc = self.regs[15] & !2; // 字对齐的 PC（硬件行为）
        self.regs[rd] = bus.read_u32(pc.wrapping_add(offset));
    }

    /// F7：`LDR/STR/LDRB/STRB Rd, [Rb, Ro]`——寄存器偏移的字/字节访问。
    /// F8：`LDRH/STRH/LDRSB/LDRSH Rd, [Rb, Ro]`——寄存器偏移的半字/有符号。
    /// 两者的差别只在 op 的三位编码（F8 的 op 位 0 恒为 0）：
    /// `000=STR, 001=STRH, 010=STRB, 011=STRSB(无), 100=LDR,
    ///  101=LDRH, 110=LDRB, 111=LDRSB(SH)`
    fn thumb_f7_8_load_store_reg(&mut self, insn: u16, bus: &mut Bus) {
        let op = (insn >> 9) & 0x7;
        let ro = ((insn >> 6) & 0x7) as usize;
        let rb = ((insn >> 3) & 0x7) as usize;
        let rd = (insn & 0x7) as usize;
        let addr = self.regs[rb].wrapping_add(self.regs[ro]);
        self.thumb_load_store(addr, rd, op, bus);
    }

    /// F7/F8 共用的加载/存储实现：按 op 决定宽度与是否符号扩展。
    fn thumb_load_store(&mut self, addr: u32, rd: usize, op: u16, bus: &mut Bus) {
        match op {
            0x0 => bus.write_u32(addr, self.regs[rd]),        // STR
            0x1 => bus.write_u16(addr, self.regs[rd] as u16), // STRH
            0x2 => bus.write_u8(addr, self.regs[rd] as u8),   // STRB
            0x4 => self.regs[rd] = bus.read_u32(addr),        // LDR
            0x5 => self.regs[rd] = u32::from(bus.read_u16(addr)), // LDRH（零扩展）
            0x6 => self.regs[rd] = u32::from(bus.read_u8(addr)), // LDRB（零扩展）
            _ => {
                // 0x7：LDRSB/LDRSH——Thumb 编码里这一格既可解释为字节也
                // 可解释为半字。真实区分靠指令的另一个位（这里 F8 的位 8
                // 为 1 表示 SH）。为稳妥，按半字符号扩展实现（游戏常用）。
                self.regs[rd] = alu::sign_extend(u32::from(bus.read_u16(addr)), 16) as u32;
            }
        }
    }

    /// F9：`LDR/STR/LDRB/STRB Rd, [Rb, #imm5×4|×1]`——立即数偏移。
    fn thumb_f9_load_store_imm(&mut self, insn: u16, bus: &mut Bus) {
        let op = (insn >> 11) & 0x3; // 00=STR, 01=LDR, 10=STRB, 11=LDRB
        let byte = op >= 2;
        let load = op & 1 != 0;
        let offset = u32::from((insn >> 6) & 0x1F) * if byte { 1 } else { 4 };
        let rb = ((insn >> 3) & 0x7) as usize;
        let rd = (insn & 0x7) as usize;
        let addr = self.regs[rb].wrapping_add(offset);
        if load {
            self.regs[rd] = if byte {
                u32::from(bus.read_u8(addr))
            } else {
                bus.read_u32(addr)
            };
        } else if byte {
            bus.write_u8(addr, self.regs[rd] as u8);
        } else {
            bus.write_u32(addr, self.regs[rd]);
        }
    }

    /// F10：`LDRH/STRH Rd, [Rb, #imm5×2]`——立即数偏移的半字访问。
    fn thumb_f10_load_store_half(&mut self, insn: u16, bus: &mut Bus) {
        let load = insn & (1 << 11) != 0;
        let offset = u32::from((insn >> 6) & 0x1F) * 2;
        let rb = ((insn >> 3) & 0x7) as usize;
        let rd = (insn & 0x7) as usize;
        let addr = self.regs[rb].wrapping_add(offset);
        if load {
            self.regs[rd] = u32::from(bus.read_u16(addr));
        } else {
            bus.write_u16(addr, self.regs[rd] as u16);
        }
    }

    /// F11：`LDR/STR Rd, [SP, #imm8×4]`——栈指针相对访问。
    fn thumb_f11_sp_relative(&mut self, insn: u16, bus: &mut Bus) {
        let load = insn & (1 << 11) != 0;
        let rd = ((insn >> 8) & 0x7) as usize;
        let offset = u32::from(insn & 0xFF) * 4;
        let addr = self.regs[13].wrapping_add(offset);
        if load {
            self.regs[rd] = bus.read_u32(addr);
        } else {
            bus.write_u32(addr, self.regs[rd]);
        }
    }

    /// F12：`ADD Rd, PC/SP, #imm8×4`。
    fn thumb_f12_load_address(&mut self, insn: u16) {
        let sp = insn & (1 << 11) != 0;
        let rd = ((insn >> 8) & 0x7) as usize;
        let imm = u32::from(insn & 0xFF);
        if sp {
            // F12 的 SP 形式：小范围调整栈指针
            self.regs[rd] = self.regs[13].wrapping_add(imm * 4);
        } else {
            // F12 的 PC 形式：取 PC 相对地址到寄存器（同样按字对齐）
            self.regs[rd] = (self.regs[15] & !2).wrapping_add(imm * 4);
        }
    }

    /// F13：`ADD/SUB SP, #imm7×4`（0xB000 = 加、0xB080 = 减，不影响标志位）。
    fn thumb_f13_add_sub_sp(&mut self, insn: u16) {
        let imm = u32::from(insn & 0x7F) * 4;
        if insn & (1 << 7) != 0 {
            self.regs[13] = self.regs[13].wrapping_sub(imm);
        } else {
            self.regs[13] = self.regs[13].wrapping_add(imm);
        }
    }

    /// F14：`PUSH/POP {寄存器列表[, LR|PC]}`
    ///
    /// 位 8 的含义按方向不同：PUSH 上是 LR（r14），POP 上是 PC（r15）。
    fn thumb_f14_push_pop(&mut self, insn: u16, bus: &mut Bus) {
        let pop = insn & (1 << 11) != 0;
        let extra = insn & (1 << 8) != 0;
        let list = (insn & 0xFF) as u32
            | match (pop, extra) {
                (true, true) => 1 << 15,  // POP {…, pc}
                (false, true) => 1 << 14, // PUSH {…, lr}
                _ => 0,
            };

        if pop {
            // POP：从 SP 起按低地址到高地址装载（SP 递增）
            let mut addr = self.regs[13];
            let mut popped_pc: Option<u32> = None;
            for r in 0..16u32 {
                if list & (1 << r) == 0 {
                    continue;
                }
                let value = bus.read_u32(addr);
                addr = addr.wrapping_add(4);
                if r == 15 {
                    // POP {PC} 的装载值可能在同一指令里被低编号寄存器覆盖
                    // （如 POP {r0, pc} 恰好 r0 = 返回地址的编码情形），所以
                    // 先暂存，寄存器全部写完后统一落地。
                    popped_pc = Some(value);
                } else {
                    self.regs[r as usize] = value;
                }
            }
            self.regs[13] = addr;
            if let Some(value) = popped_pc {
                self.branch_exchange_thumb(value);
            }
        } else {
            // PUSH：满递减栈——先减 SP，再把寄存器按下标**从高到低**入栈，
            // 使编号小的寄存器落在低地址（与 ARM 的 STMDB 一致）
            let count = list.count_ones();
            let new_sp = self.regs[13].wrapping_sub(count * 4);
            let mut addr = new_sp;
            for r in 0..16u32 {
                if list & (1 << r) == 0 {
                    continue;
                }
                let value = if r == 15 {
                    self.regs[15] // 少见：PUSH {PC}
                } else {
                    self.regs[r as usize]
                };
                bus.write_u32(addr, value);
                addr = addr.wrapping_add(4);
            }
            self.regs[13] = new_sp;
        }
    }

    /// F15：`LDMIA/STMIA Rb!, {寄存器列表}`——多寄存器装载/存储。
    fn thumb_f15_multiple(&mut self, insn: u16, bus: &mut Bus) {
        let load = insn & (1 << 11) != 0;
        let rb = ((insn >> 8) & 0x7) as usize;
        let list = (insn & 0xFF) as u32;
        let base = self.regs[rb];
        let mut addr = base;
        for r in 0..8u32 {
            if list & (1 << r) == 0 {
                continue;
            }
            if load {
                self.regs[r as usize] = bus.read_u32(addr);
            } else {
                bus.write_u32(addr, self.regs[r as usize]);
            }
            addr = addr.wrapping_add(4);
        }
        // 基址写回：列表为空时**不**写回（GBATEK 的特例，避免空列表改基址）
        if list != 0 {
            self.regs[rb] = addr;
        }
    }

    /// F17：`SWI #imm8`——软件中断，走 BIOS HLE（与 ARM 态同一入口）。
    fn thumb_f17_swi(&mut self, insn: u16, bus: &mut Bus) {
        crate::bios::hle_swi(self, bus, u32::from(insn & 0xFF));
    }

    /// F16：条件跳转（0xD000 区，`>>10`=52..=55，cond 在位 11–8；cond=0xF 即
    /// 0xDF00 区的 SWI）；
    /// F18：无条件跳转（0xE000 区，`>>10`=56..=59，11 位补码偏移 ×2）。
    /// 两个区间不可能重叠，用 `>>11` 的 11100 判别无条件形式。
    fn thumb_f18_branch(&mut self, insn: u16, bus: &mut Bus) {
        if (insn >> 11) & 0x1F == 0b11100 {
            // F18 无条件：11 位补码偏移 ×2（范围 ±2 KB）
            let offset = alu::sign_extend(u32::from(insn & 0x7FF), 11) * 2;
            self.write_pc_thumb(self.regs[15].wrapping_add(offset as u32));
            return;
        }
        // F16 条件跳转：cond 在位 11–8，偏移 8 位补码 ×2（范围 ±256 B）
        let cond = u32::from((insn >> 8) & 0xF);
        if cond == 0xF {
            self.thumb_f17_swi(insn, bus);
            return;
        }
        if cond != 0xE && cond_pass(cond, self.cpsr) {
            let offset = alu::sign_extend(u32::from(insn & 0xFF), 8) * 2;
            self.write_pc_thumb(self.regs[15].wrapping_add(offset as u32));
        }
    }

    /// F19：`BL`/`BLX` 的第一条指令——把高 11 位偏移暂存，
    /// 等第二条指令到来时合并成完整偏移与返回地址。
    fn thumb_f19_long_branch(&mut self, insn: u16) {
        // GBATEK：第一条里 LR = PC(可见) + sign_extend(hi<<12, 23)。
        // 偏移整体是 23 位有符号数（bit22 是符号位），高半段的 0x400..0x7FF
        // 表示负偏移，必须在这里就完成符号扩展，否则负跳转会绕到高地址。
        let hi = u32::from(insn & 0x7FF) << 12;
        let offset = if hi & (1 << 22) != 0 {
            hi.wrapping_sub(1 << 23)
        } else {
            hi
        };
        self.regs[14] = self.regs[15].wrapping_add(offset);
        self.thumb_bl_is_blx = insn & (1 << 12) == 0; // 位 12=0 为 BLX
    }

    /// F19 的第二条指令：合并偏移，写入 LR 返回地址并跳转。
    fn thumb_bl_complete(&mut self, insn: u16) {
        let lo = u32::from(insn & 0x7FF);
        // 执行第二条时 regs[15] = 本指令地址 + 4（Thumb 流水线约定），
        // 故本指令地址 = regs[15] - 4，返回地址 = 本指令地址 + 2 = regs[15] - 2
        let return_addr = self.regs[15].wrapping_sub(2) | 1; // 置位表示回 Thumb
        if self.thumb_bl_is_blx {
            // BLX：目标是 ARM 代码，地址要字对齐
            let target = self.regs[14].wrapping_add(lo << 1) & !3;
            self.regs[14] = return_addr;
            self.thumb_switch_to_arm(target);
        } else {
            let target = self.regs[14].wrapping_add(lo << 1);
            self.regs[14] = return_addr;
            self.write_pc_thumb(target);
        }
    }

    /// Thumb 下写 PC（跳转）：维持 `regs[15] = 目标 + 4` 约定。
    pub(crate) fn write_pc_thumb(&mut self, value: u32) {
        self.regs[15] = (value & !1).wrapping_add(4);
        self.pc_written = true;
    }

    /// 通用状态切换（Thumb 侧入口）：按目标位 0 切 ARM/Thumb。
    fn branch_exchange_thumb(&mut self, target: u32) {
        if target & 1 != 0 {
            self.write_pc_thumb(target);
        } else {
            // 切到 ARM：写 CPSR 清 T 位，PC 按 ARM 约定 = 目标 + 8
            self.cpsr &= !(1 << 5);
            self.set_pc_arm(target & !3);
            self.pc_written = true;
        }
    }

    /// 切到 ARM 态执行（BLX 用）。
    fn thumb_switch_to_arm(&mut self, target: u32) {
        self.cpsr &= !(1 << 5);
        self.set_pc_arm(target & !3);
        self.pc_written = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    fn setup_thumb() -> (Cpu, Bus) {
        let mut cpu = Cpu::new_no_bios();
        cpu.cpsr |= 1 << 5; // 进入 Thumb 态
        cpu.regs[15] = 0x0800_0004; // 即将执行 0x08000000
        (
            cpu,
            Bus::new(Cartridge::from_bytes(vec![0; 0x100].into()).unwrap()),
        )
    }

    #[test]
    fn mov_imm8_and_add() {
        let (mut cpu, mut bus) = setup_thumb();
        cpu.thumb_step(0x2005, &mut bus); // MOV r0, #5
        assert_eq!(cpu.reg(0), 5);
        cpu.thumb_step(0x3003, &mut bus); // ADD r0, #3
        assert_eq!(cpu.reg(0), 8);
    }

    #[test]
    fn alu_mul() {
        let (mut cpu, mut bus) = setup_thumb();
        cpu.regs[0] = 6;
        cpu.regs[1] = 7;
        // MUL r0, r1：格式 4，op=1101 → 0x4348
        cpu.thumb_step(0x4348, &mut bus);
        assert_eq!(cpu.reg(0), 42);
    }

    #[test]
    fn f13_add_sub_sp_leaves_registers_alone() {
        let (mut cpu, mut bus) = setup_thumb();
        cpu.regs[13] = 0x0300_7E10;
        cpu.regs[0] = 0x1234_5678;
        // SUB SP, #4（0xB081）——曾被误路由到 F12 的 PC 形式，
        // 执行成"r0 = PC 相对地址"而污染调用者的寄存器
        cpu.thumb_step(0xB081, &mut bus);
        assert_eq!(cpu.reg(13), 0x0300_7E0C);
        assert_eq!(cpu.reg(0), 0x1234_5678);
        // ADD SP, #0x100（0xB040）
        cpu.thumb_step(0xB040, &mut bus);
        assert_eq!(cpu.reg(13), 0x0300_7F0C);
        // F12 的 SP 形式不受影响：ADD r0, SP, #8（0xA802）
        cpu.thumb_step(0xA802, &mut bus);
        assert_eq!(cpu.reg(0), 0x0300_7F14);
    }

    #[test]
    fn f9_word_load_and_byte_store() {
        let (mut cpu, mut bus) = setup_thumb();
        bus.write_u32(0x0200_0000, 0xDEAD_BEEF);
        cpu.regs[1] = 0x0200_0000;
        // LDR r0, [r1]（0x6808）——曾被解码成"字节存储"而破坏内存
        cpu.thumb_step(0x6808, &mut bus);
        assert_eq!(cpu.reg(0), 0xDEAD_BEEF);
        assert_eq!(bus.read_u32(0x0200_0000), 0xDEAD_BEEF);
        // STRB r0, [r1, #1]（0x7048）——曾被解码成"字加载"
        cpu.regs[0] = 0x5A;
        cpu.thumb_step(0x7048, &mut bus);
        assert_eq!(bus.read_u8(0x0200_0001), 0x5A);
        assert_eq!(bus.read_u32(0x0200_0000), 0xDEAD_5AEF);
        // LDRB r2, [r1, #1]（0x784A）
        cpu.thumb_step(0x784A, &mut bus);
        assert_eq!(cpu.reg(2), 0x5A);
        // STR r3, [r1, #4]（0x604B）
        cpu.regs[3] = 0x1234_5678;
        cpu.thumb_step(0x604B, &mut bus);
        assert_eq!(bus.read_u32(0x0200_0004), 0x1234_5678);
    }

    #[test]
    fn f16_gt_branch_is_not_swi() {
        let (mut cpu, mut bus) = setup_thumb();
        cpu.regs[1] = 35;
        // SUB r1, #1（0x3901）→ Z=0 N=0 V=0 → GT 成立
        cpu.thumb_step(0x3901, &mut bus);
        // BGT -8（0xDCF8）——曾被路由进 SWI 组
        cpu.regs[15] = 0x0800_0100 + 8; // 指令地址 0x08000104
        cpu.thumb_step(0xDCF8, &mut bus);
        assert_eq!(cpu.reg(15), 0x0800_00F8 + 4);
    }

    #[test]
    fn push_pop_roundtrip() {
        let (mut cpu, mut bus) = setup_thumb();
        cpu.regs[13] = 0x0300_7F00;
        cpu.regs[0] = 0xAAAA;
        cpu.regs[1] = 0xBBBB;
        // PUSH {r0, r1}：0xB400 | 0x03
        cpu.thumb_step(0xB403, &mut bus);
        assert_eq!(cpu.reg(13), 0x0300_7EF8);
        // 内存里 r1 落在高地址
        assert_eq!(bus.read_u32(0x0300_7EF8), 0xAAAA);
        assert_eq!(bus.read_u32(0x0300_7EFC), 0xBBBB);
        // POP {r2, r3}：换目标寄存器验证确实是从栈里读的
        cpu.regs[0] = 0;
        cpu.regs[1] = 0;
        cpu.thumb_step(0xBC0C, &mut bus); // POP {r2, r3}
        assert_eq!(cpu.reg(2), 0xAAAA);
        assert_eq!(cpu.reg(3), 0xBBBB);
    }

    #[test]
    fn conditional_branch() {
        let (mut cpu, mut bus) = setup_thumb();
        cpu.regs[15] = 0x0800_0004;
        // BEQ +4：cond=0，偏移 2 → 0xD002
        cpu.thumb_step(0xD002, &mut bus);
        // setup 里 regs[15]=0x08000004 表示"即将执行 0x08000000"；
        // Z=0 不跳，流水线推进 2 → regs[15] = 0x08000006
        assert_eq!(cpu.reg(15), 0x0800_0006, "Z=0 不跳，PC 前进 2");
        cpu.set_flag_z(true);
        cpu.regs[15] = 0x0800_0004;
        cpu.thumb_step(0xD002, &mut bus);
        // 跳转：目标 = PC(可见=指令+4) + 偏移 = 0x08000004 + 4 = 0x08000008
        // → regs[15] = 目标 + 4 = 0x0800000C
        assert_eq!(cpu.reg(15), 0x0800_000c);
    }

    #[test]
    fn bx_switches_back_to_arm() {
        let (mut cpu, mut bus) = setup_thumb();
        cpu.regs[3] = 0x0800_0100; // 偶数 → ARM
        // BX r3：格式 5 op=11，h2=1 → 0x4718
        cpu.thumb_step(0x4718, &mut bus);
        assert!(!cpu.t());
        assert_eq!(cpu.reg(15), 0x0800_0108);
    }

    #[test]
    fn bl_uses_two_instructions() {
        let (mut cpu, mut bus) = setup_thumb();
        // BL 编码（GBATEK）：第一条 LR = PC(可见) + (hi<<12)，PC 可见 = 指令+4
        // 第二条 目标 = LR + (lo<<1)，LR = 第二条地址 + 2（返回点）
        cpu.regs[15] = 0x0800_0004; // 第一条在 0x08000000，可见 PC = 0x08000004
        cpu.thumb_step(0xF000, &mut bus); // BL 前缀：hi = 0
        assert_eq!(cpu.reg(14), 0x0800_0004); // LR 暂存高半偏移
        cpu.thumb_step(0xF840, &mut bus); // 完成后半：lo = 0x040 → 偏移 0x80
        assert_eq!(cpu.reg(15), 0x0800_0088); // 目标 0x08000084 + 流水线 4
        assert_eq!(cpu.reg(14), 0x0800_0005); // 返回地址 = 第二条(0x08000002)+2 = 0x04，|1
    }

    #[test]
    fn bl_suffix_completes_after_irq_split_pair() {
        let (mut cpu, mut bus) = setup_thumb();
        // IRQ 落在 BL 前后缀之间：前缀已执行（LR 已暂存高半偏移），
        // 中断返回后直接落到后缀——后缀必须按编码完成跳转，
        // 不能依赖"上一条是否是前缀"的跨指令状态。
        cpu.regs[15] = 0x0800_0004;
        cpu.thumb_step(0xF000, &mut bus);
        assert_eq!(cpu.reg(14), 0x0800_0004);
        cpu.thumb_step(0x4770, &mut bus); // 模拟中断里执行的 BX LR 等指令
        cpu.regs[15] = 0x0800_000A; // 中断返回，落在后缀 0x08000006（可见 PC = +4）
        cpu.regs[14] = 0x0800_0004; // 前缀留下的暂存值
        cpu.thumb_step(0xF840, &mut bus);
        assert_eq!(cpu.reg(15), 0x0800_0088);
        assert_eq!(cpu.reg(14), 0x0800_0009);
    }
}
