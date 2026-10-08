# 00 · 项目骨架：workspace、ROM 头部与帧契约

> 对应里程碑：M0　｜　预计阅读：15 分钟
> 前置要求：Rust 基本语法（struct / enum / Result / 模块系统）；硬件知识从零讲起。

## 1. 这一步做了什么

| 交付物 | 位置 | 一句话说明 |
|--------|------|-----------|
| Cargo workspace | `Cargo.toml` | 把"模拟核心"与将来的"桌面前端"拆成两个世界 |
| 卡带模块 | `crates/gba-core/src/cartridge.rs` | ROM 装载、头部解析、校验和验证（5 个单元测试） |
| 帧契约模块 | `crates/gba-core/src/frame.rs` | 定义"一帧视频"的尺寸与内存格式（2 个单元测试） |
| headless CLI | `crates/gba-core/examples/headless.rs` | 不开窗口运行核心的调试入口，含手写 BMP 编码 |
| 设计方案 | `docs/design.md` | 总体架构与里程碑（仓库内为权威版本） |

**验收结果**：`cargo build / test / fmt --check / clippy -D warnings` 全绿；
headless 实测 FireRed：标题 `POKEMON FIRE`、代码 `BPRE`、校验和通过。

## 2. 背景知识

### 2.1 GBA 卡带里有什么

实体卡带上有三类芯片，模拟器要分别对待：

| 芯片 | 作用 | 映射到的 CPU 地址 | 本项目实现于 |
|------|------|-------------------|--------------|
| ROM | 游戏程序与资源，最大 32 MiB | `0x08000000` 起 | **M0（本步）**：装载进内存 |
| 存档芯片 | SRAM / Flash / EEPROM，断电不丢 | `0x0E000000`（EEPROM 例外） | M3：`savedata` 模块 |
| GPIO（可选） | RTC 实时钟、震动等 | 卡带内一小段寄存器 | 暂不实现（FireRed 没用到） |

ROM 芯片本身就是一串字节，把它读进来毫无难度；难的是让这串字节"开口说话"。
第一步就是读懂它开头 192 字节的**头部**——相当于 ROM 的身份证。

### 2.2 ROM 头部逐字段精读（用我们自己的 FireRed）

在 Git Bash 里执行 `head -c 192 "Pokemon.gba" | od -A x -t x1z`，关键的三行：

```text
000000  7f 00 00 ea 24 ff ae 51 69 9a a2 21 ...  >....$..Qi..!...<
0000a0  50 4f 4b 45 4d 4f 4e 20 46 49 52 45 42 50 52 45  >POKEMON FIREBPRE<
0000b0  30 31 96 00 00 00 00 00 00 00 00 00 01 67 00 00  >01...........g..<
```

| 偏移 | 长度 | 值（FireRed 实测） | 含义 |
|------|------|--------------------|------|
| 0x000 | 4 | `7f 00 00 ea` | **入口跳转指令**：小端读出机器码 `0xEA00007F`，是一条 ARM `B` 无条件跳转，目标 = 指令地址 + 8（流水线，M1 详讲）+ 0x7F×4 = `0x08000204`，游戏启动代码在那里 |
| 0x004 | 156 | `24 ff ae 51 …` | **Nintendo Logo**：开机时 BIOS 校验的版权图像。我们不校验也不复制它（避免在源码里嵌入版权数据） |
| 0x0A0 | 12 | `POKEMON FIRE` | **游戏标题**（ASCII，不足补空格） |
| 0x0AC | 4 | `BPRE` | **游戏代码**：前三位是游戏标识，末位是地区（E=美/英、J=日、P=欧 …） |
| 0x0B0 | 2 | `01` | **厂商代码**（01 = Nintendo） |
| 0x0B2 | 1 | `0x96` | 固定值，头部合法性的参考标志之一 |
| 0x0B3–0xBB | 9 | 全 0 | 主机代码 / 设备类型 / 保留 |
| 0x0BC | 1 | `0x01` | **软件版本**：0=v1.0，1=v1.1 |
| 0x0BD | 1 | `0x67` | **头部校验和**（下节演算） |
| 0x0BE–0xBF | 2 | 全 0 | 保留 |

### 2.3 校验和：BIOS 的第一道安检（手工演算）

GBATEK 给出的公式，直觉理解是：**让"标题…版本"这串字节连同魔数 0x19，恰好凑成 256 的整数倍**。逐项累加：

```text
chk = 0x19                     ← 固定魔数，先入累加器
    + Σ("POKEMON FIRE") = 0x35F
    + Σ("BPRE")         = 0x129
    + Σ("01")           = 0x61
    + 0x96              （0xB2 处的固定值）
    + 0x00 × 9          （0xB3–0xBB 的保留字段）
    + 0x01              （版本）
    = 0x599
校验字节 = (−0x599) mod 256 = 0x67
```

与 ROM 在 0xBD 处烧录的 `0x67` 一致 ✓。对应代码在 `cartridge.rs` 的 `RomHeader::parse`，核心就四行：

```rust
let mut chk: u32 = 0x19;
for &b in &rom[OFF_TITLE..=OFF_VERSION] {
    chk = chk.wrapping_add(b as u32);
}
let expected = (chk.wrapping_neg() & 0xFF) as u8;
```

真机 BIOS 校验不通过就拒绝启动；模拟器**照样装载**，只把 `checksum_valid` 标成 `false` 供诊断——爱好者汉化版经常忘记修校验和，不该因此不能玩。

### 2.4 无 BIOS 启动与头部的关系（M1 剧透）

真机开机流程：BIOS 自检 → 校验 logo 与校验和 → 跳到 `0x08000000` 执行头部那条 `B` 指令 → 进入游戏。

我们的方案：没有 BIOS，开机时由 `bios.rs` 直接把 CPU 状态摆成"BIOS 刚交接完"的样子——`PC = 0x08000000`（从头部跳转指令开始执行）、各模式栈指针就位（`0x03007F00` 等）、`IME = 0`。这就是所谓 **HLE（高层模拟）**：不模拟芯片，只模拟芯片造成的结果。

### 2.5 帧契约：为什么现在就定 240×160 RGBA

M5 的前端在 WebView 里跑，核心在 Rust 线程里跑，中间隔着 IPC。两边必须先对"一帧视频长什么样"达成一致，否则每次联调都在互相猜：

- 尺寸 240×160、每像素 4 字节 RGBA、行主序（前 960 字节 = 屏幕第一行）；
- 选 RGBA8888 的私心：这是浏览器 canvas `ImageData` 的**原生格式**，前端拿到字节就能 `putImageData` 上屏，零转换。

M0 还没有 PPU，产不出真画面，所以 `Frame::test_pattern()` 先输出一张 SMPTE 彩条（白黄青绿品红蓝黑，8 条竖杠）——电视检修图的思路：**彩条能正确显示，显示链路就没问题**；三原色三间色加黑白全齐，R/B 接反一眼就能看出来。

headless 把它写成 `test-frame.bmp`。BMP 是我们唯一"手写"的编码（14 字节文件头 + 40 字节信息头 + 像素），简单到不需要图像库，核心 crate 保持零依赖。BMP 有两个反直觉的怪癖，代码里都注释了：像素行**从下往上**存，颜色序是 **B,G,R**。

## 3. 代码走读

### 3.1 目录为什么这么拆

```text
gbars/
├── Cargo.toml              # workspace：只管成员清单与公共配置
├── crates/gba-core/        # 模拟核心库（本步的全部代码）
└── docs/                   # 设计方案 + notes（你在读的这层）
```

- `gba-core` **禁止**依赖任何 UI 库——这不是洁癖：CI 里没有屏幕，核心必须能被 `cargo test` 独立验证；将来换掉 Tauri 前端，核心原封不动；
- M5 时 `apps/desktop/` 会作为第二个 workspace 成员加入，Tauri 依赖只进那里；
- workspace 根的 `[profile.release]` 已配好 `lto` 与 `codegen-units=1`（设计方案 §12）：解释器是计算密集型，发布构建值得激进优化。

### 3.2 cartridge.rs 的三个设计点

1. **错误是枚举，不是字符串**。`CartridgeError::TooSmall { size }` 把出错现场（实际大小）带在错误里；实现 `Display` + `std::error::Error` 让它融入 Rust 错误生态，`impl From<io::Error>` 让 `fs::read(path)?` 一行完成"读文件 + 错误自动包装"；
2. **库代码永不 panic**。模拟器崩一下，玩家这局进度就没了。一切可预期失败走 `Result`，由最上层（未来的 Tauri 命令层）决定弹窗还是终止；
3. **`Cartridge` 持有 `Box<[u8]>` 而非 `Vec<u8>`**：语义上强调"一整块只读的定长 ROM 数据"（无多余容量、不可追加）。16 MiB 一次性读入内存是刻意的取舍——模拟循环里每次 ROM 访问都是纯内存操作，绝不穿透文件系统。

### 3.3 frame.rs：用类型系统锁死不变式

"一帧恒为 153,600 字节"不是注释里的君子协定：`Frame` 的 `pixels` 字段私有，外部只能通过 `black()` / `test_pattern()` 构造，长度由构造函数保证。M2 的 PPU 也走同样的出口，想造出"半截帧"都过不了编译。

### 3.4 headless.rs：项目最重要的调试工具

没有它，每次验证都要开 Tauri 窗口、点开 ROM；有了它，回归就是一条命令。它的角色随里程碑成长：M0 读头部 → M1 单步执行看寄存器 → M2 之后"跑 N 帧、输出画面与哈希"做图形回归。

## 4. 如何验证

```bash
cd D:\project\rust\gbars
cargo test                                            # 7 个测试全绿
cargo clippy --workspace --all-targets -- -D warnings # 零警告
cargo run -p gba-core --example headless -- "D:/Games/gba roms/Pokemon.gba"
```

实测输出：

```text
ROM 文件:   D:/Games/gba roms/Pokemon.gba
标题:       POKEMON FIRE
游戏代码:   BPRE
厂商代码:   01
版本:       v1.1
大小:       16777216 字节 (16.00 MiB)
头部校验和: 通过
测试帧已写入 test-frame.bmp（240×160，153600 字节/帧）
```

再打开 `test-frame.bmp`：8 条竖向彩条，最左白、第 7 条蓝、最右黑。如果第 7 条显示成红色，说明颜色通道接反了——这正是测试帧存在的意义。

## 5. 下一步：M1 CPU（ARM7TDMI 解释器）

接下来是最硬、也最有意思的一仗：

- **ARM7TDMI 是什么**：ARMv4T 架构 32 位 CPU，主频 16.78 MHz。"T" 指 Thumb——同一颗 CPU 认两套指令集：ARM（定长 32 位）与 Thumb（定长 16 位，更省 ROM 空间，在 GBA 的 16 位 ROM 总线上更高效，商业游戏代码以 Thumb 为主）；
- **将新建的模块**：`cpu/`（寄存器组、模式切换、译码器、执行器）、`bus.rs`（CPU 与所有硬件之间的"主干道"：地址译码 + 内存映射）、`bios.rs`（无 BIOS 启动状态 + SWI 高层模拟）；
- **验收标准**：跑通 ArmWrestler 测试 ROM（ARM/Thumb 指令全量行为测试）。

### 延伸阅读（可选）

- GBATEK "Cartridge Header"（本步）→ "ARM7TDMI"（下一步）
- tonc 教程的 "The header" 小节
- mGBA `src/gba/cart/`（看看成熟实现的卡带抽象长什么样）

---

*看不懂的地方随时问。下一篇 notes 会假设你读过这一篇。*
