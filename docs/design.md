# GBA 模拟器设计方案

> 版本 v1.0（2026-10-08）
> 定位：从零开始的全新项目（不复用工作区内旧 `gba-emu` 项目的任何代码）
> 技术栈：Rust（模拟核心）+ Tauri 2（桌面前端壳）+ React + TypeScript + MUI（界面）
> 验收目标：运行 `D:\Games\gba roms\Pokemon.gba`（《Pokemon FireRed》）并完整游玩

---

## 1. 项目概述

### 1.1 目标

实现一个**功能完整、结构清晰、可长期演进**的 Game Boy Advance 模拟器：

- 纯 Rust 实现的模拟核心（`gba-core`），不依赖任何 UI 框架，可在无窗口环境下独立构建与测试；
- Tauri 2 桌面应用承载核心，React + MUI 提供界面（ROM 加载、按键映射、音量/缩放、暂停/重置/快进）；
- 首个兼容性验收目标：《Pokemon FireRed》可完整游玩（进标题、开新档、走路、战斗、音乐音效、存档读档）。

### 1.2 MVP 功能清单

| 功能 | 级别 |
|---|---|
| ARM7TDMI CPU（ARM + Thumb 全指令，解释器） | 必须 |
| 内存总线（全地址映射 + 等待状态） | 必须 |
| PPU 全部 6 种显示模式 + 精灵 + 窗口 + 混合 | 必须 |
| 定时器 / DMA / 中断控制器 | 必须 |
| APU（4 个 PSG 通道 + 2 个 DirectSound FIFO） | 必须 |
| Flash 128KB 存档（FireRed 所需）+ SRAM 32KB | 必须 |
| 无 BIOS 启动（HLE 引导状态 + SWI 高层模拟） | 必须 |
| EEPROM 512B/8KB 存档 | 次要（M6） |
| 快进、暂停、重置 | 必须 |
| 即时存档/读档（save state） | M6 |
| 金手指、手柄支持、着色器滤镜 | 不做（MVP） |

### 1.3 非目标（当前阶段明确排除）

- JIT / 动态重编译（解释器性能余量足够，见 §13）；
- 逐周期（cycle-accurate）级精度（目标是"商业游戏可玩"级，对标 mGBA 默认档位而非 NanoBoyAdvance）；
- 逐像素（per-dot）PPU 渲染（扫描线粒度起步，见 §6.4 限制说明）；
- 多人联机（Serial/Link Cable）、GBA → GCN 联动、回放系统。

### 1.4 代码与文档约定

> 2026-10-10 起，本项目**不再产出教学型注释与解释文档**，此前"边做边学"的约定已作废。讲解与文档统一在项目完成后由其他智能体处理。

- **不写代码注释**：源代码中不添加任何形式的注释（含模块头 `//!` 与行内注释）；
- **不写解释文档**：不产出 `docs/notes/NN-主题.md` 之类的说明文档（2026-10-10 前的三篇历史文档已删除）；
- 正确性仍由测试与实机运行（FireRed 验收）保证，只是不再靠注释/文档承载解释；
- README 仍维护里程碑进度与工程基线（`cargo fmt --check` / `cargo test` / `cargo clippy -D warnings`）。

---

## 2. 目标 ROM 分析

对 `D:\Games\gba roms\Pokemon.gba` 的实际检测结果：

| 项目 | 值 | 含义 |
|---|---|---|
| 文件大小 | 16,777,216 B（16 MiB） | 完整 ROM，无截断 |
| 标题（0xA0） | `POKEMON FIRE` | |
| 游戏代码（0xAC） | `BPRE` | **Pokemon FireRed（美版）** |
| 版本（0xBC） | 0x01 | v1.1 |
| 存档标记 | `FLASH1M_V103` | **Flash 128KB（1Mbit），需要 bank 切换** |
| RTC | 无 | FireRed/LeafGreen 不使用实时时钟（比红蓝宝石风险更低） |

**对设计的直接影响：**

1. 存档模块必须实现 **Flash 128KB**（含 bank 切换命令），SRAM 32KB 顺带实现，EEPROM 可延后；
2. 无 RTC 需求，GPIO 模块降级为"探测后返回未连接"；
3. FireRed 使用 m4a 音频引擎（DMA1/2 → FIFO + Timer0），对 DMA/定时器/中断的正确性要求高，是时序实现的试金石。

---

## 3. 参考资料

### 3.1 硬件文档

| 资料 | 用途 |
|---|---|
| **GBATEK**（Martin Korth, problemkaputt.de/gbatek.htm） | 最权威的 GBA 硬件文档：内存映射、寄存器位定义、时序表、SWI 清单、存档协议。**所有不确定的行为以 GBATEK 为准** |
| **tonc**（Jasper Vijn 的 GBA 编程教程） | PPU 背景/精灵渲染的机制讲解，比 GBATEK 更易读 |

### 3.2 本地 mGBA 源码（`D:\project\rust\mgba`，已克隆）

实现时按主题对照 mGBA 的对应文件（学习其**语义与结构**，用 Rust 重写，不做 FFI）：

| 主题 | mGBA 路径 |
|---|---|
| CPU 译码/执行分离架构 | `src/arm/arm.c`、`decoder-arm.c`、`decoder-thumb.c`、`isa-arm.c`、`isa-thumb.c` |
| 内存映射 / 等待状态 / 预取 | `src/gba/memory.c` |
| I/O 寄存器行为 | `src/gba/io.c` |
| PPU 时序与软件渲染 | `src/gba/video.c`、`src/gba/renderers/software-bg.c`、`software-mode0.c`、`software-obj.c`、`video-software.c` |
| APU（PSG + FIFO 混音） | `src/gba/audio.c` |
| 定时器 / DMA | `src/gba/timer.c`、`src/gba/dma.c` |
| 存档（Flash 命令集、EEPROM 协议、自动检测、每游戏覆写表） | `src/gba/savedata.c`、`src/gba/overrides.c` |
| **无 BIOS 启动 + SWI 高层模拟** | `src/gba/hle-bios.s`、`hle-bios.c`、`bios.c` |
| 测试套件（可自制测试 ROM） | `src/platform/test/` |

### 3.3 其他开源项目

| 项目 | 语言 | 参考价值 |
|---|---|---|
| **mizu**（github.com/Amjad50/mizu） | Rust | Rust 写的 GBA 模拟器，模块划分与 Rust 惯用法参考 |
| NanoBoyAdvance | C++ | 高精度实现，疑难行为对照 |
| VisualBoyAdvance-M | C/C++ | 兼容性边界 case 参考 |
| agb（github.com/agbrs/agb） | Rust | Rust 的 GBA homebrew 库，其示例 ROM 可作**合法测试素材** |

### 3.4 测试 ROM

| ROM | 测什么 | 来源 |
|---|---|---|
| **ArmWrestler**（修正版） | ARM/Thumb 指令集全量行为 | github.com/destoer/armwrestler-gba-fixed |
| **gba-tests** | CPU / 内存 / PPU / 定时器单项测试 | github.com/destoer/gba-tests |
| mGBA test suite | 各子系统单项 | 本地 `mgba/src/platform/test/` 编译 |
| agb examples | 实际图形/音频场景 | github.com/agbrs/agb |
| Pokemon FireRed | 最终验收 | 本地（仅用于模拟器开发兼容性验证） |

---

## 4. 总体架构

### 4.1 三层结构

```
┌─────────────────────────────────────────────────────────────┐
│  前端（React + MUI，运行在 WebView）                          │
│  ├─ Screen：canvas 显示（整数倍缩放、像素风）                  │
│  ├─ useAudio：Web Audio 播放（环形缓冲）                      │
│  ├─ 键盘监听 → set_key_state；设置界面（MUI）                 │
└──────────────┬──────────────────────────────────────────────┘
               │ Tauri IPC：命令（低频）+ Channel（高频二进制）
┌──────────────┴──────────────────────────────────────────────┐
│  Tauri 后端（Rust，薄层）                                    │
│  ├─ 模拟线程：固定 59.7275Hz 节拍驱动 gba-core               │
│  ├─ video 通道：每帧 RGBA8888（240×160）                     │
│  ├─ audio 通道：每帧 s16le 立体声块                          │
│  └─ ROM / 存档文件读写、状态持久化                            │
└──────────────┬──────────────────────────────────────────────┘
               │ 纯 Rust API（无 UI 依赖）
┌──────────────┴──────────────────────────────────────────────┐
│  gba-core（模拟核心库，可 headless 运行）                     │
│  scheduler ── cpu(arm7tdmi) ── bus ── ppu / apu / dma /      │
│  timers / interrupts / keypad / savedata / cartridge / bios  │
└─────────────────────────────────────────────────────────────┘
```

### 4.2 核心设计原则

1. **核心与界面彻底解耦**：`gba-core` 只认"帧进帧出"——输入按键位掩码进，RGBA 帧与音频采样块出。无 Tauri/窗口/音频设备依赖，可 `cargo test`、可 CLI 跑帧。
2. **单一时钟源**：所有硬件由一个统一调度器按主频 16.777216 MHz 推进，杜绝"各设备各数各的"。
3. **状态全量可快照**：核心所有状态聚合在 `GameBoyAdvance` 结构体内（无全局变量），即时存档 = 序列化该结构体。
4. **高频数据走二进制通道，低频控制走命令**：帧/音频用 Tauri 2 Channel 原始字节；按键状态每帧合并写入。

### 4.3 一帧的数据流

```
[前端] keydown/keyup ──invoke──▶ set_key_state(掩码)
[后端模拟线程] 循环 {
    core.run_frame()                    // 280,896 周期
      └─ VBlank 开始时: 拷贝帧缓冲 → video.send(RGBA帧)
      └─ APU 累积采样   → audio.send(s16le块)
    sleep 到下一帧节拍 (16.743ms，Instant 校正)
}
[前端] video.onmessage → putImageData 到 canvas
       audio.onmessage → 环形缓冲 → AudioWorklet 播放
```

---

## 5. GBA 硬件要点速览

### 5.1 内存映射

| 区域 | 地址范围 | 大小 | 总线 | 说明 |
|---|---|---|---|---|
| BIOS | 0x00000000–0x00003FFF | 16 KiB | 16 位 | 只读；我们用 HLE 桩填充 |
| EWRAM | 0x02000000–0x0203FFFF | 256 KiB | 16 位 | 慢速通用 RAM，地址位镜像 |
| IWRAM | 0x03000000–0x03007FFF | 32 KiB | 32 位 | 快速；镜像到 0x03FF8000 |
| I/O | 0x04000000–0x040003FE | ~1 KiB | 32 位 | 硬件寄存器 |
| 调色板 | 0x05000000–0x050003FF | 1 KiB | 16 位 | BG 与 OBJ 各 256 色 |
| VRAM | 0x06000000–0x06017FFF | 96 KiB | 16 位 | 0x06010000 起 OBJ 区与 BG 重叠；位镜像 |
| OAM | 0x07000000–0x070003FF | 1 KiB | 32 位 | 128 个精灵属性 + 32 组仿射参数 |
| ROM | 0x08000000–0x09FFFFFF | ≤32 MiB | 16 位 | 镜像 0x0Axxxxxx / 0x0Cxxxxxx（无预取）；未实现区域读开放总线 |
| 存档 | 0x0E000000–0x0E00FFFF | ≤128 KiB | **8 位** | SRAM/Flash；EEPROM 走 ROM 区未用地址 |

### 5.2 主时序

| 参数 | 值 |
|---|---|
| 主频 | 16.777216 MHz（2²⁴ Hz） |
| 每扫描线 | 1232 周期（HDraw 0–960，HBlank 960–1232） |
| 每帧 | 228 线（160 可见 + 68 VBlank）= **280,896 周期** |
| 帧率 | 59.7275 Hz |
| 指令周期 | ARM/Thumb 在 IWRAM 1 周期；ROM 取指受等待状态与预取影响（WAITCNT 0x04000204） |

### 5.3 子系统清单（详细设计见 §6）

- **CPU**：ARM7TDMI（ARMv4T）。ARM/Thumb 双指令集，37 寄存器（6 种特权模式分组），CPSR/SPSR，三级流水线（读 PC 时 ARM+8 / Thumb+4 偏移）。
- **PPU**：240×160，模式 0–2（图块背景，0/1 含仿射）、模式 3（16bpp 位图）、4（8bpp 双帧）、5（16bpp 双帧 160×128）；64 硬件精灵（含仿射/半透明）；两个硬件窗口 + OBJ 窗口；alpha 混合与亮度增减。
- **APU**：4 个 GB 兼容 PSG 通道（方波×2 / 波表 / 噪声）+ 2 个 DirectSound FIFO（8bit PCM，DMA 供给，定时器弹出）。
- **定时器**：4×16 位，分频 1/64/256/1024，级联模式，溢出可触发中断与 FIFO 弹出。
- **DMA**：4 通道；触发源：立即 / VBlank / HBlank / Special（DMA1→FIFO A、DMA2→FIFO B、DMA3→视频捕获）；支持重复、地址递增/固定/递减。
- **中断**：14 个源；IE(0x200) / IF(0x202，写 1 清除) / IME(0x208)；BIOS 用户跳转表在 IWRAM 顶端 0x03FFFFF0。
- **输入**：10 键（A B Select Start Right Left Up Down R L），KEYINPUT 0x130，0=按下。

---

## 6. gba-core 模块设计

### 6.1 模块划分

```text
crates/gba-core/src/
├── lib.rs            // 对外门面：GameBoyAdvance 结构体
├── scheduler.rs      // 统一时钟与事件队列
├── gba.rs            // 核心状态聚合 + run_frame 主循环
├── cpu/
│   ├── mod.rs        // CPU 状态、模式切换、异常入口、步进接口
│   ├── decode_arm.rs // ARM 译码 → 标准化指令结构
│   ├── decode_thumb.rs
│   ├── isa_arm.rs    // ARM 指令执行
│   ├── isa_thumb.rs
│   └── alu.rs        // 移位器、标志位计算（含桶形移位 carry-out）
├── bus.rs            // 地址译码、读写分发、等待状态、镜像
├── ppu/
│   ├── mod.rs        // 扫描线状态机（HDraw/HBlank/VBlank 事件）
│   ├── background.rs // 模式 0–2 文本/仿射背景
│   ├── bitmap.rs     // 模式 3–5 位图
│   ├── sprite.rs     // OAM 精灵渲染（普通/仿射/半透明）
│   ├── window.rs     // 窗口合成
│   └── blending.rs   // alpha 混合 / 亮度增减
├── apu/
│   ├── mod.rs        // 混音、采样生成、FIFO
│   ├── square.rs / wave.rs / noise.rs   // PSG 通道状态机
│   └── resample.rs   // 32.768k → 宿主采样率（线性插值）
├── timer.rs          // 定时器（含级联、FIFO 联动）
├── dma.rs            // DMA（4 通道、触发、重复）
├── interrupts.rs     // IF/IE/IME + 异常投递
├── keypad.rs         // 按键位掩码 → KEYINPUT
├── cartridge.rs      // ROM 装载、头部解析、GPIO（桩）
├── savedata/
│   ├── mod.rs        // 类型检测与分发
│   ├── sram.rs       // 32KB SRAM
│   ├── flash.rs      // 64KB/128KB Flash 命令集（含 bank 切换）
│   └── eeprom.rs     // 512B/8KB EEPROM 串行协议
├── bios.rs           // 无 BIOS 引导状态 + SWI HLE（LZ77/RL/Huffman、CpuSet 等）
└── frame.rs          // 帧输出契约（RGBA8888）
```

### 6.2 调度器（scheduler.rs）

采用 mGBA `mTiming` 的简化版：**按到期周期排序的事件队列**（二叉小顶堆）。

```rust
pub struct Scheduler {
    now: u64,                          // 主时钟周期计数
    queue: BinaryHeap<Reverse<Event>>, // (到期周期, 事件类型)
}
pub enum EventKind { PpuHDraw, PpuHBlank, PpuVBlank, Timer(usize),
                     ApuFifo(usize), DmaTrigger(usize) }

impl Scheduler {
    pub fn run_until(&mut self, target: u64, ctx: &mut dyn EventSink); // 弹出并处理所有到期事件
}
```

- CPU 每执行一条指令，把其消耗的周期数提交给调度器，由调度器依次触发到期事件；
- 各硬件不轮询，只登记"下一次事件何时到期"；
- VBlank 事件时置起中断标志并快照帧缓冲。

### 6.3 CPU（cpu/）

- **解释器**，译码与执行分离（同 mGBA）：译码产生轻量的标准化指令描述，执行层只面对归一化字段，便于逐条测试与日后替换执行策略；
- 全量 ARMv4T：数据处理（含桶形移位 carry 语义）、块传输 LDM/STM（含 S 位、PC 装载返回）、单数据传输、乘法（MUL/MLA/UMULL/UMLAL/SMULL/SMLAL）、SWP/SWPB、MRS/MSR、SWI、BX、未定义指令异常；
- 全量 Thumb（格式 1–19）；
- 流水线语义：读 R15 返回 +8/+4；写 R15 带出 CPSR 的 T 位控制；
- **周期计量**：MVP 用近似模型（指令基础周期 + 按访问区域的等待状态表），不做 N/S/I 精确管线；
- **无 BIOS 启动**（对照 GBATEK「CPU Modes」BIOS 引导后状态与 `mgba/src/gba/hle-bios.s`）：
  - `PC=0x08000000`，`CPSR=0x1F`（System 模式，IRQ 未屏蔽）；
  - `r13_sys=0x03007F00`、`r13_irq=0x03007FA0`、`r13_svc=0x03007FE0`；
  - `IME=0`、`DISPCNT=0x0080`（强制空白）、`WAITCNT=0x4317`；
- **SWI 高层模拟**（bios.rs，对照 `mgba/src/gba/bios.c`）。FireRed 强依赖项已标注：

| SWI | 名称 | FireRed 依赖 | 要点 |
|---|---|---|---|
| 0x00/0x01 | SoftReset / RegisterRamReset | ✔ | 重置入口与 RAM 清理 |
| 0x02 | Halt | ✔ | 低功耗等待 IRQ（省电轮询语义） |
| 0x03 | Stop | — | 桩实现 |
| 0x04/0x05 | IntrWait / VBlankIntrWait | ✔✔ | **游戏主循环核心**：阻塞直到指定 IF 位置起 |
| 0x06/0x07 | Div / DivArm | ✔ | 有符号除法 |
| 0x08–0x0A | Sqrt / ArcTan / ArcTan2 | ✔ | 定点数学 |
| 0x0B/0x0C | CpuSet / CpuFastSet | ✔ | 内存拷贝/填充（ROM 引导大量使用） |
| 0x0D | GetBiosChecksum | — | 恒返 0xBAAE187F |
| 0x0E/0x0F | BgAffineSet / ObjAffineSet | ✔ | 仿射参数表计算 |
| 0x10 | BitUnPack | ✔ | 位解包 |
| 0x11/0x12 | LZ77UnCompWram/Vram | ✔✔ | **Pokemon 图形资源全部依赖** |
| 0x13/0x14 | RLUnCompWram/Vram | ✔ | 行程解压 |
| 0x15 | HuffUnComp | ✔ | 霍夫曼解压 |
| 0x16–0x1F | SoundDriver 系列 / MidiKey2Freq | ✔ | m4a 引擎依赖（对照 mGBA bios.c 实现到可用程度） |

- **IRQ 进入路径（无真 BIOS）**：IRQ 异常 → 向量 0x00000018 处由 HLE 桩接管：读 IWRAM 0x03FFFFFC 取游戏 handler 地址跳转，返回后恢复。该逻辑可直接翻译 `hle-bios.s`。

### 6.4 PPU（ppu/）

- **扫描线状态机**：调度器事件驱动（每线 960 周期 HDraw 结束 → 渲染该行 → 1232 周期进 HBlank → 触发 HBlank DMA/中断；第 160 线进 VBlank）；
- 渲染顺序（每行）：窗口判定 → 4 个 BG + OBJ 按优先级合成（同优先级 OBJ 在 BG 之上）→ 半透明/alpha 混合 → 亮度增减 → RGB555→RGBA8888 写入帧缓冲；
- 文本背景（模式 0–2）：4bpp/8bpp 图块、滚动、256 色调色板；仿射背景（模式 1/2）：定点数参考坐标 + 环绕；
- 精灵：128 项 OAM、32 组仿射参数、1D/2D VRAM 映射、翻转、mosaic（后置）、OBJ 窗口；
- **已知限制（接受）**：扫描线粒度渲染，行中途写寄存器下一行才生效（HBlank DMA 在渲染前生效，故 FireRed 常规效果正确；个别逐像素特技可能异常，留 per-dot 升级路径）。

### 6.5 APU（apu/）

- PSG 四通道完整状态机（包络、扫频、长度、占空比；噪声 LFSR）；
- FIFO A/B：32 位×4 深队列，定时器溢出弹出 8bit 样本到左右声道；DMA1/2 Special 触发自动补充；
- 混音：PSG 音量比（25/50/100%）、FIFO 音量（50/100%）、SOUNDBIAS=0x200 中点；
- 后端重采样 32768 Hz → 前端 `AudioContext.sampleRate`（通常 48000），线性插值起步；
- 输出格式：**s16le 交织立体声**，每帧约 548 原始采样。

### 6.6 定时器 / DMA / 中断

- 定时器：16 位计数、4 档分频、级联（低位溢出驱动高位）、溢出中断、溢出驱动 FIFO 弹出；
- DMA：触发源判定（含 HBlank/VBlank/Special）、字/半字宽度、源/目标步进、重复与重载、完成中断；DMA 优先于 CPU 执行（同周期内打断取指），通道号小者优先；
- 中断控制器：IF 写 1 清除、IME/IE 屏蔽链、IRQ/FIQ 语义（FIQ 实际游戏不用，仍实现）。

### 6.7 存档（savedata/）

1. **类型检测**：ROM 内字符串扫描（`EEPROM_V*` / `SRAM_V*` / `SRAM_F_V*` / `FLASH_V*`(64KB) / `FLASH512_V*`(64KB) / `FLASH1M_V*`(128KB)），FireRed 命中 `FLASH1M_V103`；检测不到时回退运行时行为探测，UI 提供手动覆盖（对照 `mgba/src/gba/overrides.c` 的思路）；
2. **Flash 128KB**：命令序列（0xAA@0x5555 → 0x55@0x2AAA → 命令）、厂商 ID 读取、扇区擦除、写入、**bank 切换（0xB0 命令，0x10000 上下库）**；厂商 ID 支持 Macronix/Sanyo/Panasonic（具体 ID 与命令时序以 GBATEK「FLASH」章与 `mGBA savedata.c` 为准）；
3. **SRAM 32KB**：直通读写；
4. **EEPROM**：串行位协议，经 DMA 寄存器窗口访问；6 位/14 位寻址自动判别（M6）；
5. **持久化**：`Pokemon.sav` 存于 ROM 同目录；脏标记 + 节流写盘（每 2s 或暂停/关闭时刷盘，防止存档损坏）。

### 6.8 帧输出契约（frame.rs）

- 尺寸 240×160，格式 **RGBA8888**（字节序 R,G,B,A），单帧 153,600 B；
- 每帧恰好一份、由调用方取走所有权（`Box<[u8]>`），核心不复用未消费帧；
- APU 每帧输出 `Vec<i16>`（交织 L/R，已重采样到宿主采样率）。

---

## 7. Tauri 后端设计

### 7.1 线程模型

```
主线程（Tauri 命令处理）
  └─ 模拟线程 EmulatorThread
       拥有 GameBoyAdvance + video/audio Channel 发送端
       循环：run_frame() → 推送帧/音频 → Instant 校正 sleep 到 16.743ms
  └─ 控制队列（crossbeam）：LoadRom / Pause / Resume / Reset / Shutdown /
                            SetKeys / SetSpeed / SaveState / LoadState
       （模拟线程在每帧边界检查控制队列，避免任何核心内部加锁）
```

- 核心状态只在模拟线程内被触碰 → **无锁、无竞争**；
- `set_key_state` 只向队列投递最新位掩码（按键按帧合并，去抖动由游戏侧自行处理）；
- 快进 = 节拍间隔除以倍速（上限 4×），音频照常产出（前端加速播放或丢弃策略二选一，MVP 选丢弃）；
- 窗口关闭 → 投递 Shutdown → 模拟线程刷盘存档、join、退出。

### 7.2 IPC 协议

**命令（invoke，低频）**

| 命令 | 参数 | 返回 |
|---|---|---|
| `init_emulator` | `{ sampleRate: u32 }`（前端 AudioContext 实测值） | 绑定 video/audio/status 三个 Channel |
| `load_rom` | `{ path: String }` | `RomInfo { title, code, version, saveType, size }` |
| `pause` / `resume` / `reset` / `shutdown` | — | — |
| `set_key_state` | `{ mask: u16 }` | — |
| `set_speed` | `{ multiplier: f32 }` | — |
| `save_state` / `load_state` | `{ slot: u8 }` | — |
| `read_settings` / `write_settings` | — / `{ … }` | JSON |

**Channel（高频二进制）**

| 通道 | 载荷 | 频率 |
|---|---|---|
| `video` | 原始 RGBA8888 帧（153,600 B，Tauri 2 Channel 原始字节模式） | ~59.7/s |
| `audio` | s16le 交织立体声块（每帧约 800 采样×2 声道×2 B ≈ 3.2 KB） | ~59.7/s |
| `status` | JSON 事件：`rom_loaded` / `paused` / `error` / `save_written` | 低 |

带宽合计约 9.3 MB/s，Tauri 2 Channel 原始二进制模式可承受；若实测抖动，备选方案为自定义 URI scheme 流式拉取（fetch ReadableStream），架构上只替换传输层。

### 7.3 A/V 同步策略

- MVP：模拟线程固定节拍；前端音频用**消费速率**作为主时钟——环形缓冲目标水位 60–120ms，溢出丢旧、下溢静音；
- 迭代：前端定期回报 `AudioContext` 实际播放位置，后端 ±0.5% 微调节拍，消除长时间漂移；
- 视频不做复杂丢帧逻辑：Channel 天然覆盖旧帧（有界最新帧策略）。

---

## 8. React + MUI 前端设计

### 8.1 技术选型

| 项 | 选择 |
|---|---|
| 框架 | React 19 + TypeScript + Vite |
| UI 库 | MUI（Material UI v7+）：AppBar、Drawer、Dialog、Slider、Switch、ToggleButton |
| 状态 | zustand（轻量，避免样板） |
| 通信 | @tauri-apps/api v2（invoke + Channel） |

### 8.2 组件结构

```text
src/
├── App.tsx                  // 布局：AppBar + 主区（Canvas）+ 可折叠 Drawer
├── components/
│   ├── Screen.tsx           // 240×160 离屏 canvas → 显示 canvas 整数倍缩放
│   ├── RomPicker.tsx        // MUI Dialog + 原生文件对话框（tauri-plugin-dialog）
│   ├── EmulatorBar.tsx      // 暂停/重置/快进 ToggleButton、音量 Slider
│   ├── KeybindDialog.tsx    // 按键映射（捕获 keydown，写 settings）
│   └── StatusBar.tsx        // ROM 信息、FPS、存档状态 Snackbar
├── hooks/
│   ├── useEmulator.ts       // 封装 invoke/Channel 生命周期
│   ├── useAudio.ts          // AudioContext + AudioWorklet 环形缓冲
│   └── useInput.ts          // 键盘 → GBA 掩码（合并重复事件）
├── store/emulatorStore.ts   // zustand：运行状态、设置
└── theme.ts                 // 暗色 MUI 主题（游戏机配色）
```

### 8.3 界面草图

```
┌────────────────────────────────────────────────┐
│ ▤ GBA 模拟器   [打开ROM ▾] [⏸] [↺] [»4x]  ⚙  │  ← MUI AppBar
├────────────────────────────────────────────────┤
│                                                │
│                ┌──────────┐                    │
│                │ 240×160  │  ×3 整数缩放       │
│                │  Canvas  │  image-rendering:  │
│                │          │  pixelated         │
│                └──────────┘                    │
│                                                │
├────────────────────────────────────────────────┤
│ Pokemon FireRed (BPRE) │ 59.7 fps │ 音量 ──●── │  ← StatusBar
└────────────────────────────────────────────────┘
  ⚙ 设置 Drawer：键位映射 / 缩放倍数 / 音量 / 存档路径说明
```

- **Canvas 渲染**：`video.onmessage` → `ImageData(u8, 240,160)` → `putImageData` 到离屏 canvas → `drawImage` 到显示 canvas（`imageSmoothingEnabled=false`，整数倍缩放）；
- **音频**：首次用户点击时创建 `AudioContext`（浏览器自动播放策略），AudioWorklet 消费环形缓冲（MVP 用消息式拷贝）；
- **设置持久化**：tauri-plugin-store（JSON 于 appdata），记录键位/音量/缩放/最近 ROM；
- **错误呈现**：`status` 通道收到 `error` → MUI Snackbar，绝不静默吞错。

### 8.4 默认键位（mGBA 习惯，可改）

| GBA | 键盘 | GBA | 键盘 |
|---|---|---|---|
| A | X | Start | Enter |
| B | Z | Select | Backspace |
| L | A | 方向 | ↑↓←→ 或 WASD（双绑定） |
| R | S | | |

---

## 9. 项目目录结构（建议）

项目名暂定 **`gbars`**（可改），建在 `D:\project\rust\gbars\`：

```text
gbars/
├── Cargo.toml                # workspace：核心 + Tauri 应用
├── crates/
│   └── gba-core/             # 模拟核心（无 UI 依赖，结构见 §6.1）
│       └── examples/headless.rs   # headless CLI：跑 N 帧 → 输出 PNG/哈希（回归测试用）
├── apps/
│   └── desktop/              # Tauri 应用
│       ├── src-tauri/        # Rust 后端（§7）
│       └── (React 前端 §8)
├── docs/
│   ├── design.md            # 本设计方案（仓库内为权威版本）
│   └── HANDOFF.md           # 交接文档（现状、下一步任务）
├── tests/                    # 集成测试：测试 ROM 断言（不随仓库分发 ROM）
└── README.md
```

依赖清单：

- `gba-core`：零外部运行时依赖（可选 `log`）；`serde/derive` 仅用于 save state；
- 后端：`tauri 2`、`tauri-plugin-dialog`、`tauri-plugin-store`、`crossbeam-channel`；
- 前端：`react`、`@mui/material`、`@emotion/react`、`zustand`、`@tauri-apps/api`。

---

## 10. 开发里程碑

每个里程碑以**可验收目标**驱动，验收前不进入下一阶段；FireRed 里程碑贯穿全程作为最终标尺。

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **M0 骨架** | workspace、核心 crate（ROM 装载/头部解析、帧契约）、headless CLI、fmt+clippy+test 基线 | `cargo test` 绿；headless 装载 ROM 打印头部并输出测试帧位图 |
| **M1 CPU** | ARM+Thumb 全指令解释器、无 BIOS 启动、SWI HLE | ArmWrestler 全绿；gba-tests CPU/内存项通过 |
| **M2 图形** | 调度器 + PPU 全模式（背景/精灵/窗口/混合） | gba-tests PPU 项通过；**FireRed 进入标题画面且渲染正确** |
| **M3 系统** | 中断/定时器/DMA 完整闭环、Flash 128KB 存档 | FireRed：新游戏 → 走入世界 → 菜单 → **存档后重启可 CONTINUE** |
| **M4 音频** | APU 全通道 + 前端音频管线 | FireRed 标题音乐/战斗音效/叫声正常，无爆音失真 |
| **M5 前端整合** | Tauri 线程模型 + IPC + MUI 完整界面 | 全流程在桌面应用内完成：选 ROM、玩、调键位/音量/缩放、暂停/重置 |
| **M6 打磨** | 快进、即时存档、EEPROM、异常恢复、性能验证 | 4× 快进满帧率；连续游玩 30 分钟无崩溃无存档损坏 |

> 工期预估（全职等效）：M0–M1 约 1 周，M2 约 1 周，M3 约 1 周，M4 约 3–4 天，M5 约 3–4 天，M6 约 3 天；合计约 5 周。

---

## 11. 测试与验证策略

1. **CPU 单元测试**：每条指令族独立 `#[test]`（寄存器/标志位/内存副作用/PC）；ArmWrestler 与 gba-tests 作为集成验收；
2. **headless 回归**：`headless.rs` 跑任意 ROM N 帧输出 PNG + 帧哈希；关键断言（FireRed 标题帧）纳入 `tests/`，敏感 ROM 哈希不随仓库分发，由本地脚本验证；
3. **golden frame 对拍**：以 mGBA 为参照（可用其构建 + headless 截图功能），同 ROM 同帧数比对像素差，定位 PPU 细节偏差；
4. **音频验证**：静音测试（寄存器关闭→全零采样）+ 帧采样数守恒断言 + 人耳验收 FireRed 曲目；
5. **存档验证**：Flash 写入/擦除/bank 切换单测；FireRed 真实存档读回兼容；
6. **工程基线**：`cargo fmt --check`、`cargo clippy -D warnings`、`cargo test --workspace` 每次提交必绿。

---

## 12. 性能预估与优化路径

- 需求：16.78M 周期/s ≈ 6–8M 指令/s；
- Rust 解释器在桌面 CPU 单线程通常可达 100M+ 指令/s 量级（mGBA C 解释器实测数百 MIPS），**余量 10×+，无需 JIT**；
- 潜在热点与对策（按需启用）：
  - 总线地址译码 → 用位掩码查表替代区间 if 链；
  - PPU 行渲染 → 逐行缓存 tile 像素、`u16` 批量写帧缓冲；
  - 每帧 153KB IPC → Channel 原始字节模式；若仍抖动 → 自定义 URI scheme 流式；
  - 编译 `codegen-units=1 + LTO`。

---

## 13. 风险与应对

| 风险 | 影响 | 应对 |
|---|---|---|
| 无真 BIOS，HLE 语义偏差导致卡死 | 高 | 逐条对照 `hle-bios.s`/`bios.c`；IntrWait/Halt 语义单测覆盖 |
| 时序近似导致 FireRed 漂移/闪烁 | 中 | WAITCNT 等待状态表先行实现；HBlank DMA 生效点对齐；必要时补预取模拟 |
| Flash bank 切换实现错误 → 存档损坏 | 高 | 命令集单测 + 真实 .sav 回读验证 + 脏标记节流写盘 |
| WebView 音频延迟/爆音 | 中 | AudioWorklet + 环形缓冲水位控制；A/V 同步按 §7.3 分两步走 |
| 扫描线渲染导致个别特效异常 | 低 | 接受（FireRed 场景基本不受影响）；预留 per-dot 升级路径 |
| IPC 带宽不足 | 低 | 9.3MB/s 远低于实测上限；备选流式协议 |
| 法律 | — | 仓库不含 BIOS/ROM；ROM 与 BIOS 由用户提供（本方案明确无 BIOS 也可运行） |

---

## 14. 待确认决策点（默认按推荐执行）

| # | 决策 | 推荐 | 备选 |
|---|---|---|---|
| 1 | 项目名与位置 | `D:\project\rust\gbars` | 任意（建仓前可改） |
| 2 | BIOS 策略 | 无 BIOS HLE 启动（开箱即用） | 若你有 BIOS dump，做成可选加载提升精度 |
| 3 | 存档位置 | ROM 同目录 `Pokemon.sav` | appdata 集中管理 |
| 4 | 默认键位 | mGBA 风格（§8.4） | 任意自定义 |
| 5 | 快进时音频 | 丢弃音频 | 加速播放 |

---

## 15. 附录：关键寄存器速查

| 寄存器 | 地址 | 说明 |
|---|---|---|
| DISPCNT | 0x04000000 | 显示模式/图层开关 |
| DISPSTAT | 0x04000004 | VBlank/HBlank/VCount 状态与中断使能 |
| VCOUNT | 0x04000006 | 当前行 |
| BG0CNT–BG3CNT | 0x04000008–0x0400000E | 背景属性 |
| DMA0SAD…DMA3CNT_H | 0x040000B8–0x040000E0 | DMA |
| TM0CNT_L…TM3CNT_H | 0x04000100–0x0400010E | 定时器 |
| KEYINPUT | 0x04000130 | 按键（0=按下） |
| SOUNDCNT_L/H/X、SOUNDBIAS | 0x04000080–0x04000088 | 音频控制 |
| FIFO_A / FIFO_B | 0x040000A0 / 0x040000A4 | DirectSound |
| IE / IF / WAITCNT / IME | 0x04000200/202/204/208 | 中断与等待状态 |

---

*本方案批准后按 §10 里程碑推进：M0 从建 workspace 与核心骨架开始。*
