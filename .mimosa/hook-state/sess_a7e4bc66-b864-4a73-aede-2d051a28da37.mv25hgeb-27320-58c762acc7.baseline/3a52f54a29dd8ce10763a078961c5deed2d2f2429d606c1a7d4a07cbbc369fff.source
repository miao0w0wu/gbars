//! 卡带模块：ROM 装载与头部解析。
//!
//! # 硬件背景：GBA 卡带里有什么
//!
//! 打开一张实体 GBA 卡带，里面主要有三类芯片：
//!
//! | 芯片 | 作用 | CPU 地址空间 | 本项目实现于 |
//! |------|------|--------------|--------------|
//! | ROM | 游戏程序与资源（最大 32 MiB） | `0x08000000` 起 | 本模块（M0 装载） |
//! | 存档芯片 | SRAM / Flash / EEPROM，断电不丢数据 | `0x0E000000`（EEPROM 例外） | `savedata` 模块（M3） |
//! | GPIO | RTC 实时钟、震动马达等（可选） | 卡带内寄存器 | 暂不实现（FireRed 未用） |
//!
//! ROM 芯片本身只是一串字节；让这串字节"有意义"的第一步，是读懂开头的
//! **ROM 头部**（ROM 的"身份证"）——这是本模块的全部职责。
//!
//! # ROM 头部布局（`0x000`–`0x0BF`，共 192 字节）
//!
//! ```text
//! 偏移     长度   含义
//! 0x000    4      入口跳转指令（一条 ARM 的 B 指令，跳到游戏启动代码）
//! 0x004    156    Nintendo Logo（BIOS 开机校验用）
//! 0x0A0    12     游戏标题（ASCII，不足补空格）
//! 0x0AC    4      游戏代码（如 BPRE = Pokemon FireRed 美版）
//! 0x0B0    2      厂商代码（"01" = Nintendo）
//! 0x0B2    1      固定值 0x96
//! 0x0B3    1      主机代码（0 = GBA）
//! 0x0B4    1      设备类型
//! 0x0B5    7      保留（全 0）
//! 0x0BC    1      软件版本（0 = v1.0，1 = v1.1 …）
//! 0x0BD    1      头部校验和
//! 0x0BE    2      保留（全 0）
//! 0x0C0    …      多引导（multiboot）下载代码区（不属于头部）
//! ```
//!
//! 字段的逐字节精读与校验和的手工演算，见 `docs/notes/00-project-skeleton.md`。
//!
//! # 参考
//!
//! - GBATEK "Cartridge Header" 章节（字段权威定义）
//! - mGBA `src/gba/cart/`（成熟实现的卡带抽象，M3 再对照）

use std::fmt;
use std::fs;
use std::path::Path;

/// ROM 头部大小：`0x000`–`0x0BF` 共 192 字节。
pub const HEADER_SIZE: usize = 0xC0;

/// 游戏标题字段偏移（12 字节 ASCII）。
const OFF_TITLE: usize = 0x0A0;
/// 游戏代码字段偏移（4 字节 ASCII）。
const OFF_GAME_CODE: usize = 0x0AC;
/// 厂商代码字段偏移（2 字节 ASCII）。
const OFF_MAKER_CODE: usize = 0x0B0;
/// 软件版本字段偏移（1 字节）。
const OFF_VERSION: usize = 0x0BC;
/// 头部校验和字段偏移（1 字节）。
const OFF_CHECKSUM: usize = 0x0BD;

/// 从 ROM 字节装载/解析卡带时可能发生的错误。
///
/// 设计约定（见 docs/notes/00 §3.2）：库代码**永不 panic**——
/// 模拟器崩一下会丢玩家进度，一切可预期失败都走 `Result`，
/// 由最上层（未来的 Tauri 命令层）决定如何呈现给用户。
#[derive(Debug)]
pub enum CartridgeError {
    /// 文件读取失败（路径不存在、无权限等），由标准库透传。
    Io(std::io::Error),
    /// ROM 长度连 192 字节的头部都放不下，显然不是合法 GBA ROM。
    TooSmall { size: usize },
}

impl fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CartridgeError::Io(e) => write!(f, "读取 ROM 文件失败: {e}"),
            CartridgeError::TooSmall { size } => {
                write!(
                    f,
                    "ROM 太小（{size} 字节），放不下 {HEADER_SIZE} 字节的头部"
                )
            }
        }
    }
}

// 实现 std::error::Error 让它成为"合格的 Rust 错误"：
// 可以被 ? 传播、被 boxed、带 source 链。
impl std::error::Error for CartridgeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CartridgeError::Io(e) => Some(e),
            _ => None,
        }
    }
}

// 有了这个转换，`fs::read(path)?` 就能在返回 CartridgeError 的函数里
// 一行完成"读文件 + 错误自动包装"，这是 Rust 错误处理的惯用法。
impl From<std::io::Error> for CartridgeError {
    fn from(e: std::io::Error) -> Self {
        CartridgeError::Io(e)
    }
}

/// 解析后的 ROM 头部（只保留对模拟器有意义的字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomHeader {
    /// 游戏标题，如 `"POKEMON FIRE"`。
    pub title: String,
    /// 游戏代码，如 `"BPRE"`。惯例：前三位是游戏标识，末位是地区
    /// （E = 美/英、J = 日、P = 欧 …）。
    pub game_code: String,
    /// 厂商代码，`"01"` = Nintendo。
    pub maker_code: String,
    /// 软件版本：0 表示 v1.0，1 表示 v1.1，以此类推。
    pub version: u8,
    /// 头部校验和是否通过。
    ///
    /// 真机 BIOS 校验失败会拒绝启动；模拟器照样装载，只做标记——
    /// 改版 ROM（如汉化补丁）常忘记修校验和，不该因此不能玩。
    pub checksum_valid: bool,
}

impl RomHeader {
    /// 从 ROM 字节解析头部。`rom` 长度必须 ≥ [`HEADER_SIZE`]。
    pub fn parse(rom: &[u8]) -> Result<Self, CartridgeError> {
        if rom.len() < HEADER_SIZE {
            return Err(CartridgeError::TooSmall { size: rom.len() });
        }

        let title = ascii_field(&rom[OFF_TITLE..OFF_TITLE + 12]);
        let game_code = ascii_field(&rom[OFF_GAME_CODE..OFF_GAME_CODE + 4]);
        let maker_code = ascii_field(&rom[OFF_MAKER_CODE..OFF_MAKER_CODE + 2]);
        let version = rom[OFF_VERSION];

        // 头部校验和公式（GBATEK）：
        //   chk = 0x19 + sum(字节 0xA0..=0xBC)          ← 0x19 是固定魔数
        //   0xBD 处应存的值 = (-chk) 取低 8 位
        // 直觉理解：让"标题..版本 + 魔数"这一串字节恰好凑成 256 的整数倍。
        // 魔数 0x19 的来源没有公开资料解释，属于 Nintendo 的固定约定。
        // （用 FireRed 真实字节的手工演算见 docs/notes/00 §2.3）
        let mut chk: u32 = 0x19;
        for &b in &rom[OFF_TITLE..=OFF_VERSION] {
            chk = chk.wrapping_add(b as u32);
        }
        let expected = (chk.wrapping_neg() & 0xFF) as u8;
        let checksum_valid = rom[OFF_CHECKSUM] == expected;

        Ok(Self {
            title,
            game_code,
            maker_code,
            version,
            checksum_valid,
        })
    }
}

impl fmt::Display for RomHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}) v1.{}", self.title, self.game_code, self.version)
    }
}

/// 解析头部中的定长 ASCII 字段：
/// 可见 ASCII 原样保留，其余字节替换为 `?`，并去掉尾部填充的空格。
///
/// 防御式解析——字段理论上都是 ASCII，但我们不信任文件内容，
/// 绝不让畸形字节在解析阶段引发 panic 或产生不可读的乱码。
fn ascii_field(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '?'
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// 一张已装载的卡带：完整 ROM 字节 + 解析好的头部。
///
/// 后续里程碑中，CPU 将通过总线以地址 `0x08000000 + 偏移` 访问 `rom`；
/// 存档芯片（M3）也会挂到这个结构上。
pub struct Cartridge {
    /// ROM 原始字节。用 `Box<[u8]>` 而非 `Vec<u8>` 是刻意的：
    /// 它语义上是"一整块只读的定长数据"（没有多余容量、不可追加）。
    rom: Box<[u8]>,
    header: RomHeader,
}

// 手写 Debug 而不是 #[derive(Debug)]：派生版本会把 16 MiB 的 ROM 字节
// 全部打印出来（日志/测试失败信息会爆炸）。只输出长度与头部即可。
// 顺带满足测试里 `unwrap_err()` 对成功类型 T: Debug 的要求。
impl fmt::Debug for Cartridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cartridge")
            .field("rom_len", &self.rom.len())
            .field("header", &self.header)
            .finish()
    }
}

impl Cartridge {
    /// 从文件路径装载卡带，例如 `D:\Games\gba roms\Pokemon.gba`。
    ///
    /// 整个 ROM（最大 32 MiB）一次性读入内存：模拟循环里每次 ROM 访问
    /// 都是纯内存操作，绝不穿透文件系统——这是性能上必须的取舍。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CartridgeError> {
        let rom = fs::read(path)?;
        Self::from_bytes(rom.into())
    }

    /// 从内存构造卡带（单元测试，以及将来从即时存档恢复时使用）。
    pub fn from_bytes(rom: Box<[u8]>) -> Result<Self, CartridgeError> {
        let header = RomHeader::parse(&rom)?;
        Ok(Self { rom, header })
    }

    /// 已解析的头部信息。
    pub fn header(&self) -> &RomHeader {
        &self.header
    }

    /// ROM 原始字节（即地址 `0x08000000` 起映射的内容）。
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 《Pokemon FireRed》美版 v1.1 头部 `0xA0`–`0xBF` 的真实字节，
    /// 作为测试向量（对照 docs/notes/00 §2.2 的 hex dump 讲解）。
    const FIRE_RED_HEADER_TAIL: [u8; 32] = [
        // 0xA0: 标题 "POKEMON FIRE"
        b'P', b'O', b'K', b'E', b'M', b'O', b'N', b' ', b'F', b'I', b'R', b'E',
        // 0xAC: 游戏代码 "BPRE"
        b'B', b'P', b'R', b'E', // 0xB0: 厂商代码 "01"
        b'0', b'1', // 0xB2: 固定值 0x96；0xB3 主机代码；0xB4 设备类型
        0x96, 0x00, 0x00, // 0xB5–0xBB: 保留（7 字节全 0）
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        // 0xBC: 版本 v1.1；0xBD: 校验和 0x67；0xBE–0xBF: 保留
        0x01, 0x67, 0x00, 0x00,
    ];

    /// 用上面的尾部字节拼出一份 192 字节的最小 ROM。
    fn fire_red_rom() -> Vec<u8> {
        let mut rom = vec![0u8; HEADER_SIZE];
        rom[OFF_TITLE..].copy_from_slice(&FIRE_RED_HEADER_TAIL);
        rom
    }

    #[test]
    fn parses_fire_red_header() {
        let cart = Cartridge::from_bytes(fire_red_rom().into()).unwrap();
        let h = cart.header();
        assert_eq!(h.title, "POKEMON FIRE");
        assert_eq!(h.game_code, "BPRE");
        assert_eq!(h.maker_code, "01");
        assert_eq!(h.version, 1);
    }

    /// 校验和 0x67 是按公式算出来的（演算过程见 docs/notes/00 §2.3），
    /// 这里验证我们的实现与真机 ROM 中烧录的值一致。
    #[test]
    fn fire_red_checksum_matches() {
        let cart = Cartridge::from_bytes(fire_red_rom().into()).unwrap();
        assert!(cart.header().checksum_valid);
    }

    #[test]
    fn wrong_checksum_is_detected_but_not_fatal() {
        let mut rom = fire_red_rom();
        rom[OFF_CHECKSUM] ^= 0xFF; // 故意破坏校验字节
        let cart = Cartridge::from_bytes(rom.into()).unwrap();
        // 校验失败只做标记，不拒绝装载（改版 ROM 兼容策略）
        assert!(!cart.header().checksum_valid);
    }

    #[test]
    fn rejects_rom_shorter_than_header() {
        let err = Cartridge::from_bytes(vec![0u8; 100].into()).unwrap_err();
        assert!(matches!(err, CartridgeError::TooSmall { size: 100 }));
    }

    #[test]
    fn non_ascii_bytes_are_replaced() {
        let mut rom = fire_red_rom();
        rom[OFF_TITLE] = 0xFF; // 标题首字节换成非 ASCII
        let h = RomHeader::parse(&rom).unwrap();
        assert!(h.title.starts_with('?'));
    }
}
