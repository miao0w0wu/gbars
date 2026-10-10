//! 桶形移位器与算术标志位：ARM 数据处理的数学心脏。
//!
//! # 硬件背景：桶形移位器（Barrel Shifter）
//!
//! ARM 的数据处理指令在"寄存器 → ALU"之间塞了一个能在**单周期内**移任意
//! 位数的硬件——桶形移位器。这意味着 `ADD r0, r1, r2, LSL #3`（r0 = r1 +
//! r2×8）一条指令就完成乘 8 再相加。编译器疯狂利用这个特性，所以移位器
//! 的语义必须精确，尤其是**移位器的进位输出**：
//!
//! | 操作 | 移位量 | C 的来源 |
//! |------|--------|----------|
//! | LSL | 0 | 不变（没移位） |
//! | LSL | 1..=31 | 被移出的最后一位 = 原值的位 (32-n) |
//! | LSL | 32 | 原值位 0 |
//! | LSR | 0（编码 0）| **按 32 处理**：C = 原值位 31 |
//! | LSR | 1..=31 | 原值位 (n-1) |
//! | ASR | 0（编码 0）| **按 32 处理**：C = 符号位 |
//! | ROR | 0（编码 0）| **RRX**：33 位带进位循环右移 1 位 |
//! | ROR | 1..=31（寄存器移位量 & 31 == 0 且非 0）| C = 原值位 31 |
//!
//! "编码 0"的行是寄存器移位最容易踩的坑：指令里的 5 位立即数字段
//! 表示不了 32，于是约定 0 代表 32——但这个约定**只对立即数编码成立**，
//! 寄存器指定的移位量 `Rs & 0xFF == 0` 就是真的不移位。[`shift_imm`]
//! 与 [`shift_reg`] 两个入口的存在就是为了把这个区别锁进类型里。

/// 移位类型（指令位 6-5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShiftType {
    Lsl,
    Lsr,
    Asr,
    Ror,
}

impl From<u32> for ShiftType {
    fn from(bits: u32) -> Self {
        match bits {
            0 => ShiftType::Lsl,
            1 => ShiftType::Lsr,
            2 => ShiftType::Asr,
            _ => ShiftType::Ror,
        }
    }
}

/// 立即数移位：`imm5` 是指令位 11-7 的 5 位字段。
/// 编码约定：LSL 0 = 不移；LSR/ASR 0 = 移 32 位；ROR 0 = RRX。
/// 返回 (结果, 进位输出)。
pub(crate) fn shift_imm(value: u32, ty: ShiftType, imm5: u32, carry_in: bool) -> (u32, bool) {
    match ty {
        ShiftType::Lsl => lsl(value, imm5, carry_in),
        ShiftType::Lsr => lsr(value, if imm5 == 0 { 32 } else { imm5 }, carry_in),
        ShiftType::Asr => asr(value, if imm5 == 0 { 32 } else { imm5 }, carry_in),
        ShiftType::Ror => {
            if imm5 == 0 {
                rrx(value, carry_in)
            } else {
                ror(value, imm5)
            }
        }
    }
}

/// 寄存器移位：`amount` = Rs 低 8 位，0 表示**不移位**（与立即数编码不同）。
pub(crate) fn shift_reg(value: u32, ty: ShiftType, amount: u32, carry_in: bool) -> (u32, bool) {
    if amount == 0 {
        return (value, carry_in);
    }
    match ty {
        ShiftType::Lsl => lsl(value, amount, carry_in),
        ShiftType::Lsr => lsr(value, amount, carry_in),
        ShiftType::Asr => asr(value, amount, carry_in),
        ShiftType::Ror => ror(value, amount),
    }
}

/// 数据处理立即数操作数的旋转：rot = 字段值 × 2（保证非 0，范围 2..=30）。
pub(crate) fn rotate_imm_carry(value: u32, rot: u32) -> (u32, bool) {
    ror(value, rot)
}

fn lsl(v: u32, n: u32, c_in: bool) -> (u32, bool) {
    match n {
        0 => (v, c_in),
        1..=31 => (v << n, ((v >> (32 - n)) & 1) == 1),
        32 => (0, v & 1 == 1),
        _ => (0, false), // > 32：什么都没剩下
    }
}

fn lsr(v: u32, n: u32, c_in: bool) -> (u32, bool) {
    match n {
        0 => (v, c_in),
        1..=31 => (v >> n, ((v >> (n - 1)) & 1) == 1),
        32 => (0, v >> 31 == 1),
        _ => (0, false),
    }
}

fn asr(v: u32, n: u32, c_in: bool) -> (u32, bool) {
    let signed = v as i32;
    match n {
        0 => (v, c_in),
        1..=31 => ((signed >> n) as u32, ((v >> (n - 1)) & 1) == 1),
        _ => ((signed >> 31) as u32, signed < 0), // ≥32：符号位填满
    }
}

fn ror(v: u32, n: u32) -> (u32, bool) {
    let r = n % 32;
    if r == 0 {
        // 整数圈：值不变，进位 = 最高位（GBATEK 对寄存器 ROR 的规定）
        (v, v >> 31 == 1)
    } else {
        (v.rotate_right(r), ((v >> (r - 1)) & 1) == 1)
    }
}

/// RRX：把 {C, value} 当 33 位数循环右移 1 位。
fn rrx(v: u32, c_in: bool) -> (u32, bool) {
    ((v >> 1) | (u32::from(c_in) << 31), v & 1 == 1)
}

/// 加法 `a + b + c`：返回 (结果, 进位输出 C, 有符号溢出 V)。
///
/// 进位用两步 `overflowing_add` 合成：先加 b 再加速度位，
/// 任一步溢出即最终进位。V 的公式：两个操作数同号且结果异号时溢出。
pub(crate) fn add_with_carry(a: u32, b: u32, c: bool) -> (u32, bool, bool) {
    let (r1, o1) = a.overflowing_add(b);
    let (r2, o2) = r1.overflowing_add(u32::from(c));
    let carry = o1 || o2;
    let overflow = (a ^ !b) & (a ^ r2) & (1 << 31) != 0;
    (r2, carry, overflow)
}

/// 减法 `a - b - borrow`：返回 (结果, C, V)，**C = 1 表示无借位**。
///
/// ARM 把减法的 C 定义成"无借位"（而非"有借位"），这样 SBC（带借位减）
/// 的进位链方向才一致——这是实现多位数减法的自然推论。
pub(crate) fn sub_with_borrow(a: u32, b: u32, borrow: bool) -> (u32, bool, bool) {
    let (r1, u1) = a.overflowing_sub(b);
    let (r2, u2) = r1.overflowing_sub(u32::from(borrow));
    let no_borrow = !(u1 || u2);
    let overflow = (a ^ b) & (a ^ r2) & (1 << 31) != 0;
    (r2, no_borrow, overflow)
}

/// 把 `bits` 位宽的值符号扩展到 32 位（分支偏移等压缩字段用）。
pub(crate) fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsl_carry_comes_from_last_shifted_out_bit() {
        assert_eq!(lsl(0x8000_0008, 4, true), (0x0000_0080, false));
        assert_eq!(lsl(0x1000_0008, 4, true), (0x0000_0080, true));
    }

    #[test]
    fn encoded_zero_means_32_for_lsr_and_asr() {
        assert_eq!(shift_imm(0x8000_0000, ShiftType::Lsr, 0, false), (0, true));
        assert_eq!(
            shift_imm(0x8000_0000, ShiftType::Asr, 0, false),
            (0xFFFF_FFFF, true)
        );
        assert_eq!(shift_imm(0x4000_0000, ShiftType::Asr, 0, false), (0, false));
        // 但 LSL 0 就是"不移位"，进位保持
        assert_eq!(
            shift_imm(0x1234_5678, ShiftType::Lsl, 0, true),
            (0x1234_5678, true)
        );
    }

    #[test]
    fn ror_zero_means_rrx() {
        assert_eq!(shift_imm(3, ShiftType::Ror, 0, true), (0x8000_0001, true));
    }

    #[test]
    fn register_shift_zero_means_no_shift() {
        assert_eq!(shift_reg(0xFF, ShiftType::Lsr, 0, true), (0xFF, true));
        // 寄存器 ROR 32：整圈，C = bit31
        assert_eq!(
            shift_reg(0x8000_0001, ShiftType::Ror, 32, false),
            (0x8000_0001, true)
        );
        // 移位量 > 32 的 LSR：结果 0，无进位
        assert_eq!(shift_reg(0xFF, ShiftType::Lsr, 40, false), (0, false));
    }

    #[test]
    fn add_flags() {
        assert_eq!(add_with_carry(0xFFFF_FFFF, 1, false), (0, true, false));
        assert_eq!(
            add_with_carry(0x7FFF_FFFF, 1, false),
            (0x8000_0000, false, true)
        );
        assert_eq!(add_with_carry(0xFFFF_FFFF, 0, true), (0, true, false));
    }

    #[test]
    fn sub_flags() {
        assert_eq!(sub_with_borrow(5, 5, false), (0, true, false));
        assert_eq!(sub_with_borrow(0, 1, false), (0xFFFF_FFFF, false, false));
        assert_eq!(
            sub_with_borrow(0x8000_0000, 1, false),
            (0x7FFF_FFFF, true, true)
        );
        assert_eq!(sub_with_borrow(0, 0, true), (0xFFFF_FFFF, false, false));
    }

    #[test]
    fn sign_extend_works() {
        assert_eq!(sign_extend(0x00FF_FFFF, 24), -1);
        assert_eq!(sign_extend(0x0000_0004, 24), 4);
        assert_eq!(sign_extend(0x0000_07FE, 12), 2046);
        assert_eq!(sign_extend(0x0000_0800, 12), -2048);
    }
}
