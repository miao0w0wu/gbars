# 交接文档（HANDOFF）

> 写给接手的 AI 或开发者：读完这一篇即可无缝继续。
> 最后更新：2026-10-11（M2 收尾状态：PPU 验收已过，FireRed 引导卡在随机种子循环，根因已定位）。

## 0. 一句话现状

GBA 模拟器项目 **gbars** 位于 `D:\project\rust\gbars\`。
**M0/M1 已提交**；M2 图形主体已在 `9078372` 提交；本次会话在 `e85a92d`（仓库清理）
与 `22148de`（9 处真 bug 修复 + DMA 触发补齐 + 24 个新测试）落地。
**`cargo test` 83 全绿、clippy `-D warnings` 与 fmt 全绿。**

- **ArmWrestler 菜单画面渲染正确**（标题/署名/测试菜单/橙色光标，M2 PPU 验收核心缺口已闭合）；
- **FireRed 跑 300 帧不崩**，但引导停在 0x081E3568 的"写 0x83 → 轮询 [0x04000118] 直到 > 0x1F"
  随机种子循环——根因已定位：**未映射 IO 区缺"写入暂存 + 读回"的开总线行为**
  （mGBA 把 16 位写统一存进 io[] 数组、读回原值；我们丢弃写、读恒 0），下一步补上即可继续推进。

工作区干净，以上全部已提交。HEAD = `22148de`，未推送（origin/main 在 `9078372`）。

## 1. 项目背景与约定（必须遵守）

| 项 | 内容 |
|---|---|
| 目标 | 用 Rust 写 GBA 模拟器（核心纯库）+ Tauri 2 + React + MUI 前端，最终完整运行 `D:\Games\gba roms\Pokemon.gba`（**Pokemon FireRed 美版 v1.1**，BPRE，存档类型 Flash 128KB/`FLASH1M_V103`，无 RTC） |
| 设计方案 | `docs/design.md`（权威版；架构、IPC 协议、M0–M6 里程碑表） |
| **代码/文档约定（用户硬性要求，2026-10-10 起）** | **① 不写任何代码注释（含模块头 `//!` 与行内注释）；② 不写解释文档，不再产出 `docs/notes/`。** 既有注释/文档保留不动。正确性由测试与 FireRed 验收保证 |
| 工程基线（每次交付必须全绿） | `cargo fmt --check`；`cargo test`（当前 83 个）；`cargo clippy --workspace --all-targets -- -D warnings` |
| 工具链 | rustc/cargo 1.99.0、edition 2024、node v24、pnpm 12 |
| mGBA 源码 | 已克隆在 `D:\project\rust\mgba`（`src/gba/video.c`、`io.c`、`dma.c`、`timer.c`、`sio.c`、`renderers/video-software.c`、`include/mgba/internal/gba/dma.h`、`io.h`） |
| **环境坑 1** | 安全钩子**禁止用 bash（sed/echo 等）直接写源码文件**，必须用 Write/Edit 工具 |
| **环境坑 2** | Git Bash 里 `cargo run ... \| head` 偶发吞 stdout——重定向到文件再 grep 可靠 |

## 2. 目录与文件地图

```text
gbars/
├── Cargo.toml                       # workspace（release: lto+codegen-units=1）
├── README.md                        # 里程碑进度 + 工程基线命令
├── docs/
│   ├── design.md                    # 设计方案（权威）
│   └── HANDOFF.md                   # ← 本文
├── test-roms/armwrestler.gba        # CPU 测试 ROM
└── crates/gba-core/src/
    ├── lib.rs                       # 门面 + 模块地图
    ├── cartridge.rs                 # ROM 装载与头部解析（M0）
    ├── frame.rs                     # 帧契约 240×160 RGBA8888（M0）
    ├── bus.rs                       # 内存总线 + IRQ 桩（0x18 = b 0x120）+ run_dma_triggers
    ├── gba.rs                       # GameBoyAdvance 门面 + run_frame()
    ├── bios.rs                      # SWI HLE（CpuSet fill 已改真语义）
    ├── scheduler.rs                 # 统一时钟；run_frame/run_instruction/now()
    ├── io.rs                        # 0x04000000 寄存器层；SIOCNT 已挂 0x128
    ├── timer.rs                     # 4×16 位定时器（分频/级联/溢出 IRQ）
    ├── dma.rs                       # 4 通道 DMA（立即/VBlank/HBlank、repeat、dst 重载）
    ├── ppu/
    │   ├── mod.rs                   # PPU 主体；frame555 整帧行缓冲
    │   ├── background.rs            # 文本（screen_base 按半字）+ 仿射背景
    │   ├── bitmap.rs                # 模式 3/4/5
    │   ├── sprite.rs                # OBJ
    │   ├── blending.rs              # 亮度/alpha 混合
    │   └── window.rs                # 窗口（全关 = 0x3F 全放行）
    └── cpu/
        ├── mod.rs                   # 模式/银行/异常/step()
        ├── alu.rs                   # 桶形移位器与标志位
        ├── isa_arm.rs               # ARM 全指令（category_000 判别已修正）
        └── isa_thumb.rs             # Thumb 全 19 格式
```

## 3. 本次会话完成的工作

### 3.1 仓库卫生与基线（e85a92d）
- `9078372` 误把 `.mimosa/`（插件会话产物）和临时 `frame*.bmp` 提交进仓库，已移除并加入 .gitignore；
- 修掉 dma.rs 两处 `unnecessary_cast` 与全部 fmt diff，恢复绿基线。

### 3.2 修复九处真 bug（22148de）——按因果链排序

| # | 文件 | 缺陷 | 后果/证据 |
|---|---|---|---|
| 1 | `cpu/isa_arm.rs` `category_000` | 位 7、4=1 的杂烩区**先判位 24**：后索引（P=0，位 24=0）的 LDRH/STRH 被当 UMULL 执行，基址寄存器被乘积清零 | ArmWrestler 字体 blitter 的 `LDRH r1,[r10],r2` 把 r10 清 0 → 所有 VRAM/调色板写落到地址 0 → 纯黑画面。**正确判别：先看位 6:5**——`00` = 乘法（位 24=0）或 SWP（位 24=1）；`01/10/11` = 半字族（与位 24 无关）。旧 HANDOFF §4.3 记的"位 24=0 → 乘法"就是这条错误规则，已废弃 |
| 2 | `ppu/mod.rs` | `capture_frame` 对 160 行全部调 `write_line_to_frame(y)`，但 `ppu.line` 只保存最后渲染的一行（VBlank 期 160–227 行不渲染） | 整帧 = 第 159 行复制 160 份。新增 `frame555: Vec<u16>`（240×160），`render_line` 完即留档 |
| 3 | `ppu/window.rs` | 三个窗口使能全关时用 WINOUT（复位值 0）做全屏掩码 → 所有图层被拒 | 只剩 palette[0] 黑底。真机/mGBA（video-software.c `control.packed = 0xFF`）：窗口全关 = 全放行，WINOUT 不参与 |
| 4 | `dma.rs` 步进 | `delta(bits) * unit / 2`：半字传输每单元只走 1 字节 | 传输尾寄存器 0x0200_0101（应为 0x102）。正确：`delta * unit`，unit 已是字节数 |
| 5 | `ppu/background.rs` 文本 BG | `screen_base = SC * 0x800`（字节）后又 `map_offset * 2`，screenbase≠0 时地址翻倍 | screenbase=0 的测试掩盖了它。文本路径 entry 读是半字，`screen_base` 应按 `SC * 0x400`；仿射路径按字节读、保持 `* 0x800` 不变——**两条路径单位不同，别"统一"** |
| 6 | `bus.rs` IRQ 桩 | 0x18 向量用 `LDR PC,[PC,#-8]`：0x18 处可见 PC=0x20，-8 读到**自己这条指令**的字节 → 跳到 0xE51FF008 执行垃圾 | FireRed 第一次 IRQ 后 CPU 在野地址流浪（PC 每帧 +0x1124F0）。改为 mGBA hle-bios.s 原文：`b irqBase` = 0xEA000040（0x18 → 0x120） |
| 7 | `bios.rs` CpuSet/CpuFastSet | fill 模式把 **r0 寄存器值**当填充值 | GBATEK/真机：fill 读 **[r0] 处内存**一次作为填充值。FireRed 初始化 IO 块（0x04000208 起 20 个半字）时被灌进指针垃圾，IE/IF/WAITCNT 大面积损坏 |
| 8 | `io.rs` SIOCNT | 读写都挂在 **0x134**（RCNT 的地址）；0x128 无处理，读恒 0、写落 PPU 杂项；且无 start 位自动清零 | FireRed 写 SIOCNT=0x83 后轮询 start 位清零，永不退出。现在：0x128 读写接通、移位完成（2048 周期）清 bit7 并投递 SIO IRQ（IF bit7） |
| 9 | `dma.rs` ch1/2 计数 | count=0 展开成 0x400 | GBATEK：ch1/2 = 0x4000。顺带补齐：repeat（触发型保持使能、仅 dst 模式 3 重载、count 寄存器即重载值）、完成 IRQ（ctrl bit14 → IF bit8-11）、立即触发永不 repeat |

### 3.3 补齐 VBlank/HBlank DMA 触发（原 M2 清单声称有、实际没有）
- `Scheduler::advance`：可见行行尾调 `bus.run_dma_triggers(2)`（HBlank），第 160 行调 `run_dma_triggers(1)`（VBlank）——对齐 mGBA `_startHblank`/`_startVblank`（HBlank DMA 只在 0–159 行，渲染发生在 HBlank 起点，与我们"行尾渲染"近似一致）；
- `Bus::run_dma_triggers(timing)` 统一执行并投递完成 IRQ；立即 DMA 完成同样投递。

### 3.4 新增公共 API 与 24 个测试（59 → 83 全绿）
- `Scheduler::run_instruction(cpu, bus)`：单指令 + 全部硬件副作用（定时器/IO/扫描线），
  `Scheduler::now()` 读帧内周期——**逐指令调试的工具底座**（见 §5）；
- 回归测试：后索引半字写回 + UMULL/SWP 判别（isa_arm）、整帧捕获、窗口全放行、
  模式 0 文本（含 screenbase≠0）、模式 4 双页、OBJ 同优先级覆盖；
- timer 5 个（分频/累加相位/使能边沿重载/级联/级联忽略分频）；
- dma 5 个（立即拷贝/字宽固定 dst/count=0 展开/repeat 存活/dst 模式 3 重载）；
- scheduler 7 个（227 行 + DISPSTAT=0x0003/VBlank 标志在 160 行翻转/IRQ 门控/HBlank IRQ/
  定时器溢出 IRQ/HBlank DMA 每行 160 次/VBlank DMA 第 160 行一次）。

### 3.5 FireRed 引导排查实录（未完，详见 §6.1）

- 300 帧冒烟不再 panic；第 0 帧末 PC=0x081E3BF6（Thumb，ROM，crt0 正常）；
- crt0 经 SWI 跳转表（0x081E3BD0=CpuFastSet/0x081E3BD4=CpuSet/0x081E3BD8=Div…，
  每项 `SWI NN; BX LR`）逐段复制 IWRAM/WRAM，BL 目标与返回地址经验证**正确**；
- 当前卡点：0x081E3568–0x081E3570 的 Thumb 循环
  `MOV r0,#0x83; STRH r0,[r4]; LDRH r0,[r4]; CMP r0,#0x1F; BLS 回`，
  **r4 = 0x04000118**（r5=0x3F，r6/r7=0）。真 SIOCNT(0x128)=0x5001 已配好
  （Normal 模式/内部时钟/IRQ 使能），IE=0x0080、IF=0x0081、IME 此刻=0；
- 0x04000118 在 mGBA `io.h` 与 GBATEK 里都不是命名寄存器（0x110–0x11E 属未映射区）。
  游戏把它当"写完立刻能读回"的便签：写 0x83 后立刻读回（真机开总线 ≈ 刚驱动的值），
  0x83 > 0x1F 退出循环作为随机种子。我们丢弃写、读恒 0 → 死循环；
- **修复方案（下一步第一条）**：io.rs 为 0x04000000 窗口加一块原始 u16 暂存
  （所有 16 位写先落数组，未命中专用寄存器的读回数组值；mGBA 语义），
  注意别破坏现有寄存器的专用读路径。

## 4. 关键技术与"血泪教训"（改代码前必读）

1. **PC 流水线硬约定**（全项目最重要）：
   `regs[15]` 在执行指令期间恒等于"读 PC 的硬件可见值"：ARM = 指令地址 + 8；Thumb = 指令地址 + 4。
   取指地址 = `regs[15] - 8`（ARM）/ `- 4`（Thumb）。分支落地必须写 `目标 + 8/+4`（`write_pc_arm/thumb` 是唯一出口）。
   **调试采样 PC 时同样要减偏移**——本次排查两次因把 reg(15) 当指令地址而误判
   （"卡在 0x4EC 循环"实为 0x4DC 循环；SWI 检测差 4 字节险些冤枉 BL 译码）。
2. **Thumb 译码按 `insn >> 10`（10 位组）**：F1 移位组占 `0..=5`、F2 加减占 `6..=7`、F9 占 `24..=31`。
3. **ARM 杂烩区（位 7、4 同时为 1）判别（本次实测修正）**：**先判位 6:5**
   ——`00` = 乘法族（位 24=0）或 SWP/SWPB（位 24=1）；`01/10/11` = LDRH/LDRSB/LDRSH 半字族，
   **前索引与后索引都走这里**（后索引位 24=0，绝不能按位 24 优先分流，否则半字传输被当乘法执行、
   基址寄存器被清零）。旧文档"位 24 = 0 → 乘法"的判法已证实是错的。
4. **块传输统一算法**：寄存器恒按编号升序从 low 向上排；写回按 GBATEK 的 ULD 规则。
5. **错误策略**：合法但未实现 → `CpuError::Unimplemented`；真机未定义编码 → `enter_exception(0x04, Undefined)`。
6. **测试期望值必须手工演算**。本次 12 个新测试初跑失败全部是测试自身错：
   编码位段写错（0xE0D1_30B2 的位 22=1 是立即数偏移不是寄存器；UMULL 的 RdLo 在位 15:12、
   RdHi 在位 19:16，位 7:4 必须 = 1001）、screenbase 编码错（0x0100 不是 0x0800）、
   repeat 位（0x0200）误进非 repeat 用例、
   **IO 地址字面量位数**：`0x0400_0C0` 只有 7 个十六进制位 = 0x004000C0，总线静默丢弃——
   必须写 `0x0400_00C0`（下划线按 4 位一组数）。
7. **总线怪癖**（`bus.rs`）：调色板/VRAM 背景区 8 位写复制进整个半字（OBJ 区不复制）；
   OAM 忽略 8 位写；SRAM 宽读字节重复；ROM 窗口外读 0（16MiB ROM + 0x01FFFFFF 窗口）。
8. **M2 架构决定**：调色板/VRAM/OAM 移入 `Ppu` 持有；IRQ 桩以静态机器码注入 BIOS 内存
   （0x18 向量 = `b 0x120`，翻译自 mGBA `hle-bios.s`）；改 `bus.rs`/`cpu/mod.rs` 时别破坏。
9. **调度近似**：整行事件近似——HBlank 标志行尾置起、HDraw/HBlank 不细分 960/272；
   定时器按指令粒度推进。依赖精确 HBlank 时机的光栅特效会有偏差。
10. **DMA 单位**：`delta(bits) * unit` 就是字节数，没有 /2；count=0 的展开 ch0=0x40000、
    ch1/2=0x4000、ch3=0x10000；repeat 触发型传完不关使能、dst 仅模式 3 重载
    （基址在使能边沿捕获到 `dst_base`）、count 寄存器本身即重载值。
11. **SWI 跳转表调试法**：FireRed 的 SWI 包装在 0x081E3BD0 起每 4 字节一项（`SWI NN; BX LR`），
    逐指令跟踪时先确认执行地址再断言"调用了哪个 SWI"；CpuSet 的 r2：位 0-20 计数、
    位 24 = fill（fill 读 [r0] 一次）、位 26 = 32 位。
12. **排查顺序的方法论（本次验证有效）**：先看"数据到底写没写进去"（VRAM/调色板/IWRAM 非零字节计数）
    → 再看"谁该写而没写"（逐指令跟踪寄存器演化）→ 最后才是渲染路径。
    `Scheduler::run_instruction` + 帧回卷（`now() >= CYCLES_PER_FRAME` 时 `reset_frame()`）
    可以做带时序的逐指令跟踪；临时探针放 `crates/gba-core/examples/probe.rs`，用完即删（本次已删）。

## 5. 验证与运行命令

```bash
cd D:\project\rust\gbars
cargo test                 # 83 通过（当前基线）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# ArmWrestler 600 帧 → frame.bmp（现在应渲染出菜单文字）
cargo run -p gba-core --example headless -- test-roms/armwrestler.gba 600 frame.bmp

# FireRed 300 帧 → firered.bmp（当前仍纯色：引导未到开屏）
cargo run -p gba-core --example headless -- "D:/Games/gba roms/Pokemon.gba" 300 firered.bmp

# 单步跟踪（无时序，仅 CPU）
cargo run -p gba-core --example headless -- test-roms/armwrestler.gba --trace 200000
```

看 BMP 内容的土办法（无 PIL）：24 位 BMP 直接偏移 54 字节读像素、按 BGR 统计颜色数；
或手写 zlib 转 PNG 后用 Read 工具看图（本次用它确认了 ArmWrestler 菜单渲染正确）。

## 6. 下一步任务清单（按顺序）

1. **未映射 IO 的开总线行为**（FireRed 引导当前卡点，方案见 §3.5）：
   io.rs 加原始 u16 暂存数组，未命中专用寄存器的读写走"写即存、读即取"；
   跑 FireRed 冒烟确认随机种子循环退出、引导继续推进。
2. **继续 FireRed 引导**直到 DISPCNT 脱离 0x0080、IWRAM 0x03003580 出现 handler 代码、
   进入 IntrWait 主循环，最终 300 帧内出标题画面（M2 验收）。
   已知后续雷区：IntrWait/VBlankIntrWait 真语义已具备；中断返回路径（IRQ 桩 + SUBS PC,LR,#4）
   在桩修复后尚未被 FireRed 实际走通过；handler 复制到 IWRAM 的动作尚未发生（boot 很早期）。
3. **mGBA 对拍**（旧清单 §6.2c 的遗留项）：构建 `D:\project\rust\mgba` headless 截图，
   同 ROM 同帧对拍像素差，定位 PPU 细节偏差（窗口/混合/马赛克还没被真实 ROM 压测过）。
4. **M3 系统**：中断控制器完善、KEYINPUT（现恒 0x3FF）、Flash 128KB 存档。
   验收 = FireRed 新游戏 → 走入世界 → 存档重启可 CONTINUE。
5. 按需更新 `README.md` 里程碑与本文件；**继续遵守"不写注释/不写解释文档"约定**。

## 7. 未完成/已知限制总表

| 项 | 状态 | 解决时机 |
|---|---|---|
| FireRed 引导停在 RNG 种子循环（未映射 IO 开总线缺失） | **当前卡点** | 立即（§6.1） |
| 未映射 IO 读恒 0（与上同源） | 待修 | 立即 |
| HBlank 精确时机（960/272 细分）、HBlank DMA 时序 | 整行近似 | M3/M6 按需 |
| DMA 触发方式 3（声音 FIFO/视频捕获） | 未实现 | M4 音频 |
| GPIO/RTC、EEPROM、SRAM/Flash 存档 | 未开始 | M3 |
| APU 音频 | 未开始 | M4 |
| 按键输入（KEYINPUT 恒 0x3FF 全未按） | 未开始 | M3 |
| Huffman 仅 bits ∈ {2,4,8}、BitUnPack 位宽 8/16/32 | M1 遗留限制 | 按需 |
| mGBA 同帧对拍 | 未做 | M2 收尾 |

## 8. 参考资源

- GBATEK（problemkaputt.de/gbatek.htm）：硬件行为最终依据（直连与 WebFetch 都 403，
  需要时可用 mGBA 源码宏定义交叉验证，本次 DMA 位域即如此定案）；
- 本地 mGBA 源码 `D:\project\rust\mgba`：`video.c`（行事件/HBlank 时点）、
  `renderers/video-software.c`（窗口全关 = 0xFF）、`dma.c` + `include/mgba/internal/gba/dma.h`
  （DMA 位域与 repeat/reload 语义）、`sio.c`（SIO 完成清 start 位 + IRQ）、
  `hle-bios.s`（IRQ 桩原文：`b irqBase`）；
- 测试 ROM：`test-roms/armwrestler.gba`；gba-tests 系列可从 `destoer/gba-tests`（GitHub）获取；
- 项目约定：不写解释文档、不写代码注释（历史遗留注释保留不动）。
