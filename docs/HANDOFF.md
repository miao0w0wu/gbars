# 交接文档（HANDOFF）

> 写给接手的 AI 或开发者：读完这一篇即可无缝继续。
> 最后更新：2026-10-10（M2 中期状态）。

## 0. 一句话现状

GBA 模拟器项目 **gbars** 位于 `D:\project\rust\gbars\`。
**M0（骨架）与 M1（ARM/Thumb 全指令 + SWI HLE）已完成并提交**；
**M2（图形）的主体代码已写完但未提交**：调度器、PPU 全模式渲染管线、定时器、DMA、
IO 寄存器层、中断投递（含 BIOS IRQ 桩）均已落地，`cargo test` 59 个测试全绿，
但 **clippy/fmt 基线当前是红的（2 处 cast 警告 + 1 处格式），且 PPU 输出画面为纯色
（渲染内容未出来），M2 新模块一个单元测试都没写**。

git 已有 4 次提交（HEAD = `4fdc80b`），上述 M2 工作全部停留在工作区（9 个修改文件
+ 6 个新增文件）。

## 1. 项目背景与约定（必须遵守）

| 项 | 内容 |
|---|---|
| 目标 | 用 Rust 写 GBA 模拟器（核心纯库）+ Tauri 2 + React + MUI 前端，最终完整运行 `D:\Games\gba roms\Pokemon.gba`（**Pokemon FireRed 美版 v1.1**，BPRE，存档类型 Flash 128KB/`FLASH1M_V103`，无 RTC） |
| 设计方案 | `docs/design.md`（权威版；架构、IPC 协议、M0–M6 里程碑表） |
| **代码/文档约定（用户硬性要求，2026-10-10 起）** | **① 不写任何代码注释（含模块头 `//!` 与行内注释）；② 不写解释文档，不再产出 `docs/notes/`。** 此前"中文教学注释 + 每步五段说明文档"的约定已作废。`docs/notes/` 下既有文档与现有代码注释**原样保留、不回头清理**；讲解工作由用户在项目完成后交给其他智能体统一处理 |
| 工程基线（每次交付必须全绿） | `cargo fmt --check`；`cargo test`；`cargo clippy --workspace --all-targets -- -D warnings` |
| 工具链 | rustc/cargo 1.99.0、edition 2024、node v24、pnpm 12 |
| mGBA 源码 | 已克隆在 `D:\project\rust\mgba`（实现时按主题对照：`src/arm/` CPU、`src/gba/bios.c` SWI HLE、`src/gba/memory.c` 总线、`src/gba/hle-bios.s` 引导状态） |
| **环境坑** | 本环境有安全钩子：**禁止用 bash（sed/echo 等）直接写源码文件**，必须用 Write/Edit 工具 |

## 2. 目录与文件地图

```text
gbars/
├── Cargo.toml                       # workspace（release: lto+codegen-units=1）
├── README.md                        # 里程碑进度 + 工程基线命令
├── docs/
│   ├── design.md                    # 设计方案（权威）
│   └── HANDOFF.md                   # ← 本文
├── test-roms/armwrestler.gba        # CPU 测试 ROM（16,936 B，非商业）
└── crates/gba-core/src/
    ├── lib.rs                       # 门面 + 模块地图
    ├── cartridge.rs                 # ROM 装载与头部解析（M0）
    ├── frame.rs                     # 帧契约 240×160 RGBA8888（M0）
    ├── bus.rs                       # 内存总线 + IRQ 桩注入（M1/M2）
    ├── gba.rs                       # GameBoyAdvance 门面 + run_frame()（M1/M2）
    ├── bios.rs                      # SWI HLE（M1；IntrWait 已改真语义）
    ├── scheduler.rs                 # ★M2 新增：统一时钟与扫描线调度
    ├── io.rs                        # ★M2 新增：0x04000000 I/O 寄存器层（含 SIO 移位）
    ├── timer.rs                     # ★M2 新增：4×16 位定时器（分频/级联）
    ├── dma.rs                       # ★M2 新增：4 通道 DMA（立即/VBlank/HBlank 触发）
    ├── ppu/
    │   ├── mod.rs                   # ★M2 新增：PPU 主体（寄存器/扫描线缓存）
    │   ├── background.rs            # ★M2 新增：文本/仿射背景渲染
    │   ├── bitmap.rs                # ★M2 新增：模式 3/4/5 位图渲染
    │   ├── sprite.rs                # ★M2 新增：OBJ 渲染
    │   ├── blending.rs              # ★M2 新增：亮度/alpha 混合
    │   └── window.rs                # ★M2 新增：窗口区域判定
    └── cpu/
        ├── mod.rs                   # 模式/银行/异常/PC 约定/step()（M2 补周期模型+中断投递）
        ├── alu.rs                   # 桶形移位器与标志位
        ├── isa_arm.rs               # ARM 全指令
        └── isa_thumb.rs             # Thumb 全 19 格式（F1/F9 译码边界已修正）
```

## 3. 已完成的工作

### M0（骨架）——已提交
ROM 载入 + 头部解析（`cartridge.rs`）、帧输出契约（`frame.rs`）、headless 示例、工程基线。

### M1 CPU（ARM/Thumb 全指令 + SWI HLE）——已提交
- CPU 为解释器，译码与执行分离；ARMv4T 全量 ARM 与 Thumb（19 格式）。
- 无 BIOS 启动状态按 GBATEK/`hle-bios.s` 设定（PC=0x08000000、CPSR=0x1F、
  各模式 SP、DISPCNT=0x0080、WAITCNT=0x4317）。
- SWI HLE：`SoftReset`、`RegisterRamReset`、`Div/DivArm`、`Sqrt`、`ArcTan(2)`、
  `CpuSet/CpuFastSet`、`BgAffineSet/ObjAffineSet`、`BitUnPack`、LZ77/RL/Huffman 解压、
  `MidiKey2Freq`；Sound driver 系列（0x16–0x1E）no-op。
- 已知实现局限：Huffman 仅支持 bits ∈ {2,4,8}、尾部不足 4 字节未写回；
  BitUnPack 仅支持目标位宽 8/16/32。

### M2 图形（工作区未提交，主体已完成）
- **调度器**（`scheduler.rs`）：280,896 周期/帧（228 线 × 1232），CPU 逐指令执行、
  沿扫描线边界渲染并推进定时器；DISPSTAT 的 VBlank/HBlank 标志与对应中断按行投递。
- **周期模型**：CPU `charge/take_cycles`，取指按 `Bus::access_cycles` 计费，
  数据访问周期由总线累计（`take_data_cycles`）；Halt 睡眠期按 1 周期/步推进。
- **PPU**（`ppu/`）：DISPCNT/DISPSTAT/VCOUNT/BG 控制与滚动/仿射参数/窗口/混合寄存器
  全量接进 `io.rs`；文本背景（4bpp/8bpp、多屏基）、仿射背景（参考点每线推进）、
  位图模式 3/4/5、OBJ（普通/仿射、半透明）、窗口、亮度/alpha 混合各成一体；
  逐线渲染进 `line/line_layer/line_second` 缓存，`capture_frame` 整帧拷入 `Frame`。
- **中断投递**：IO 层维护 IE/IF/IME 与 `request_irq/clear_irq`；CPU 每步之后
  `service_irq`（IME 开 && IE&IF≠0 且 I 标志未屏蔽 → 清 Halt → 进入 IRQ 异常）。
  BIOS 内存注入了 mGBA `hle-bios.s` irqBase 的机器码桩（0x18 向量 → 0x120 入口：
  保存寄存器 → 经 0x03FFFFFC 调游戏 handler → SUBS PC,LR,#4 返回），所以"无 BIOS"
  也能真跑中断返回路径。IntrWait/VBlankIntrWait 已从 no-op 改为
  "清 IF → 挂起（Halt）→ 等中断唤醒"的真语义。
- **定时器**（`timer.rs`）：4×16 位、分频 1/64/256/1024、级联、溢出中断投递。
- **DMA**（`dma.rs`）：4 通道、立即/VBlank/HBlank 触发、源/目标步进、半字/字；
  `write_ctrl` 判定立即传输后由总线取走执行。
- **IO 层**（`io.rs`）：0x04000000 窗口的寄存器读写分派 + SIO 内部时钟移位
  （FireRed 引导靠 SIO 中断做随机种子，启动位写入后 256 周期/位移 8 位置 IF）。
- **Thumb 译码修正**：F1 移位组原判 `0..=1` 漏了 LSR/ASR（应为 `0..=5`），
  F9 立即数偏移组原判 `24..=27` 漏了 `28..=31`（应为 `24..=31`）。
- **headless 示例扩展**：`--trace N` 单步调试输出；跑 N 帧输出 BMP + PPU/CPU 状态摘要；
  `trace-io` feature（`Cargo.toml`）可打开 IO 写入追踪。

### 当前实测状态（2026-10-10）
- `cargo test`：59 通过（与 M1 基线相同；**M2 新模块无任何测试**）。
- `cargo clippy -D warnings`：**失败**——`dma.rs:66/67` 两处
  `unnecessary_cast`（`unit as i32` 中 unit 已是 i32）。
- `cargo fmt --check`：**有 diff**——`examples/headless.rs:128` 附近一行格式。
- ArmWrestler 600 帧冒烟：不 panic、不卡死，PC=0x0800055c、4159 万步、
  DISPCNT=0x0404（模式 4）、VCOUNT 正常推进；**但输出 BMP 为纯色单帧**
  （当前代码输出纯黑，历史产物 frame300/600.bmp 是纯白——渲染内容未出来，
  应显示屏幕测试文字）。重复运行输出逐字节一致（确定性 OK）。
- 根目录 `frame300.bmp`/`frame600.bmp` 为临时产物，未入 .gitignore，待清理。

## 4. 关键技术与"血泪教训"（改代码前必读）

1. **PC 流水线硬约定**（全项目最重要）：
   `regs[15]` 在执行指令期间**恒等于"读 PC 的硬件可见值"**：
   ARM = 指令地址 + 8；Thumb = 指令地址 + 4。
   取指地址 = `regs[15] - 8`（ARM）/ `- 4`（Thumb）。
   **分支落地必须写 `目标 + 8/+4`** 以维持该约定（`write_pc_arm` / `write_pc_thumb` 是唯一出口）。
2. **Thumb 译码按 `insn >> 10`（10 位组）**：F1 移位组占 `0..=5`、F2 加减占
   `6..=7`、F9 占 `24..=31`——按"9 位组 + 单独位 10"判会漏格（M1 调试实录，
   曾导致 LSR/ASR 与 STRH 区静默跑错）。
3. **`category_000` 的判别（位 7、4 同时为 1 的杂烩区）**：
   位 24 = 0 → 乘法；否则再看位 6-5：`00` = SWP/SWPB；`01/10/11` = LDRH/LDRSB/LDRSH。
   绝不能只判一位。
4. **块传输统一算法**：寄存器恒按编号升序从 low 向上排；写回按 GBATEK 的 ULD 规则。
5. **错误策略**：合法但未实现 → `CpuError::Unimplemented`；真机未定义编码 →
   `enter_exception(0x04, Undefined)`。两者语义不同。
6. **测试期望值必须手工演算**。历史上失败多为测试自己写错——先怀疑测试再怀疑实现。
7. **总线怪癖**（`bus.rs`）：调色板/VRAM 背景区 8 位写复制进整个半字（OBJ 区不复制）；
   OAM 忽略 8 位写；SRAM 宽读字节重复；镜像掩码规则；ROM 跨 0x0A 段镜像偏移。
8. **M2 架构决定**：调色板/VRAM/OAM 移入 `Ppu` 持有（渲染器直读，总线转发并保留
   位宽怪癖处理）；IRQ 桩以静态机器码注入 BIOS 内存（翻译自 mGBA `hle-bios.s`），
   中断返回路径是真执行的指令而非回调——改 `bus.rs`/`cpu/mod.rs` 时注意别破坏。
9. **调度近似**：当前按"整行事件"近似——HBlank 标志在行尾才置起、HDraw/HBlank
   不再细分 960/272 周期；定时器在指令粒度推进。对多数游戏够用，
   但依赖精确 HBlank 时机的效果（如某些光栅特效）会有偏差。

## 5. 验证与运行命令

```bash
cd D:\project\rust\gbars
cargo test                 # 59 通过（当前基线；M2 模块待补测试）
cargo clippy --workspace --all-targets -- -D warnings   # 当前 2 处失败（dma.rs cast）
cargo fmt --check          # 当前 1 处失败（headless.rs）

# 跑 N 帧输出 BMP + 状态摘要（冒烟/回归入口）
cargo run -p gba-core --example headless -- test-roms/armwrestler.gba 600 frame.bmp

# 单步跟踪前 N 条指令（M2 调试用）
cargo run -p gba-core --example headless -- test-roms/armwrestler.gba --trace 200000

# 无 ROM：输出 SMPTE 彩条 BMP（帧契约链路验证）
cargo run -p gba-core --example headless
```

## 6. 下一步任务清单（按顺序）

1. **恢复绿基线**：修 `dma.rs:66/67` 的 `unnecessary_cast`（删掉 `as i32`）与
   `headless.rs` 的格式 diff；跑 `cargo fmt` / `clippy` / `test` 全绿。
2. **让 PPU 画出内容**（M2 验收核心缺口）：ArmWrestler 是模式 4 位图 + 自绘文字，
   当前输出纯色说明 VRAM/调色板写入或 DISPCNT 位图渲染路径未生效。排查顺序：
   a. 用 `--trace` + `trace-io` feature 看 VRAM（0x06000000+）与 DISPCNT 的写入
      是否到达 PPU、DISPCNT 值是否含位图模式位；
   b. 核对模式 4 的 8bpp 调色板读取与页翻转（DISPCNT 位 4）；
   c. 对拍 mGBA 同 ROM 同帧截图（`D:\project\rust\mgba` 可构建 headless 截图）。
3. **给 M2 模块补测试**：scheduler（行边界/VCOUNT/DISPSTAT）、timer（分频/级联/
   溢出中断）、dma（触发方式/步进）、ppu（模式 0 文本、模式 4 位图、窗口/混合）。
   工程约定：期望值手工演算，不加注释。
4. **提交这批 M2 代码**（建议拆分：调度器+周期模型 / PPU+IO / 定时器+DMA+中断）。
5. **FireRed 冒烟**：`cargo run -p gba-core --example headless -- "D:/Games/gba roms/Pokemon.gba" 300`，
   M2 验收标准 = **进入标题画面且渲染正确**。
6. **进入 M3**：中断控制器完善（Halt/IntrWait 唤醒已具备雏形）、按键 KEYINPUT、
   EEPROM/Flash 128KB 存档。验收 = 新游戏 → 走入世界 → 存档重启可 CONTINUE。
7. 按需更新 `README.md` 里程碑进度与本文件。

> 注意：**不要再写解释文档、不要加代码注释**（新约定见 §1）。

## 7. 未完成/已知限制总表

| 项 | 状态 | 解决时机 |
|---|---|---|
| PPU 渲染输出纯色（内容未出来） | **M2 核心缺口** | 立即（见 §6.2） |
| M2 模块（scheduler/ppu/timer/dma/io）测试 | 零测试 | M2 收尾（§6.3） |
| HBlank 精确时机（960/272 细分）、HBlank DMA 时序 | 整行近似 | M3/M6 按需 |
| DMA 触发方式 3（声音 FIFO/视频捕获） | 未实现 | M4 音频 |
| Halt 唤醒 | 已接入 `service_irq`（受 I 标志/IME 限制） | M3 完善 |
| GPIO/RTC、EEPROM、SRAM/Flash 存档 | 未开始 | M3 |
| APU 音频 | 未开始 | M4 |
| 按键输入（KEYINPUT 恒 0x3FF 全未按） | 未开始 | M3 |

## 8. 参考资源

- GBATEK（problemkaputt.de/gbatek.htm）：一切硬件行为的最终依据；
- 本地 mGBA 源码 `D:\project\rust\mgba`（`src/gba/video.c` PPU、`src/gba/io.c` IO、
  `src/gba/dma.c` DMA、`src/gba/timer.c` 定时器）；
- 测试 ROM：`test-roms/armwrestler.gba`；gba-tests 系列可从 `destoer/gba-tests`（GitHub）获取；
- 项目约定：不写解释文档（历史 `docs/notes/` 已删除），代码注释只保留改动前已有的。
