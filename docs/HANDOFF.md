# 交接文档（HANDOFF）

> 写给接手的 AI 或开发者：读完这一篇即可无缝继续。
> 最后更新：2026-10-10。

## 0. 一句话现状

GBA 模拟器项目 **gbars** 位于 `D:\project\rust\gbars\`。
**M0（骨架）与 M1 的 CPU 部分（ARM/Thumb 全指令 + SWI HLE）已完成并全部测试通过
（59 通过，`cargo fmt --check` / `cargo clippy -D warnings` 零警告）。
当前状态：M1 待做**可选的 ArmWrestler 冒烟验收**；下一步进入 **M2（调度器 + PPU 图形）**。**

git 已初始化并有 3 次提交，工作区干净（`65e4178` 为 HEAD）。

## 1. 项目背景与约定（必须遵守）

| 项 | 内容 |
|---|---|
| 目标 | 用 Rust 写 GBA 模拟器（核心纯库）+ Tauri 2 + React + MUI 前端，最终完整运行 `D:\Games\gba roms\Pokemon.gba`（**Pokemon FireRed 美版 v1.1**，BPRE，存档类型 Flash 128KB/`FLASH1M_V103`，无 RTC） |
| 设计方案 | `docs/design.md`（权威版；架构、IPC 协议、M0–M6 里程碑表） |
| **代码/文档约定（用户硬性要求，2026-10-10 起）** | **① 不写任何代码注释（含模块头 `//!` 与行内注释）；② 不写解释文档，不再产出 `docs/notes/`。** 此前"中文教学注释 + 每步五段说明文档"的约定已作废。`docs/notes/` 下既有文档与现有代码注释**原样保留、不回头清理**；讲解工作由用户在项目完成后交给其他智能体统一处理 |
| 工程基线（每次交付必须全绿） | `cargo fmt --check`；`cargo test`（当前 **59 个**）；`cargo clippy --workspace --all-targets -- -D warnings` |
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
│   ├── HANDOFF.md                   # ← 本文
│   └── notes/                       # 历史学习文档，不再新增：
│       ├── 00-project-skeleton.md   # M0：头部解析/校验和演算/BMP
│       ├── 01-memory-bus-and-arm-core.md   # 总线怪癖/银行/移位器/调试实录
│       └── 02-block-transfers-and-thumb.md # LDM/STM/Thumb 全集/调试实录
├── test-roms/armwrestler.gba        # CPU 测试 ROM（16,936 B，非商业）
└── crates/gba-core/src/
    ├── lib.rs                       # 门面 + 模块地图
    ├── cartridge.rs                 # ROM 装载与头部解析（M0）
    ├── frame.rs                     # 帧契约 240×160 RGBA8888（M0）
    ├── bus.rs                       # 内存总线（M1）
    ├── gba.rs                       # GameBoyAdvance 门面 + 集成测试（M1）
    ├── bios.rs                      # SWI 高层模拟（M1）
    └── cpu/
        ├── mod.rs                   # 模式/银行/异常/PC 约定/step()
        ├── alu.rs                   # 桶形移位器与标志位
        ├── isa_arm.rs               # ARM 全指令
        └── isa_thumb.rs             # Thumb 全 19 格式
```

## 3. 已完成的工作

### M0（骨架）
罗马载入 + 头部解析（`cartridge.rs`）、帧输出契约（`frame.rs`）、headless 示例、工程基线。

### M1 CPU（ARM/Thumb 全指令 + SWI HLE）
- CPU 为**解释器**，译码与执行分离；ARMv4T 全量 ARM 与 Thumb（19 格式）。
- **无 BIOS 启动状态**已按 GBATEK/`hle-bios.s` 设定（PC=0x08000000、CPSR=0x1F、各模式 SP、DISPCNT=0x0080、WAITCNT=0x4317）。
- **SWI HLE**（`bios.rs`，已接入 ARM 与 Thumb 两条 SWI 路径，**不再触发异常入口、不进 Supervisor**）：
  - 已实现：`SoftReset`、`RegisterRamReset`、`Div/DivArm`、`Sqrt`、`ArcTan/ArcTan2`、
    `CpuSet/CpuFastSet`、`GetBiosChecksum`、`BgAffineSet/ObjAffineSet`、`BitUnPack`、
    `LZ77 解压（Wram/Vram）`、`RL 解压`、`Huffman 解压`、`MidiKey2Freq`；
    Sound driver 系列（0x16–0x1E）按 no-op。
  - **有意留桩（后续里程碑补）**：
    - `Halt`：置 `cpu.halted = true`（`step()` 直接返回），唤醒逻辑待 **M3 中断控制器**；
    - `IntrWait/VBlankIntrWait`：**目前立即返回（no-op）**。M3 改为"清 IF → 挂起 → 中断唤醒"真语义——官方 BIOS 会先吐掉已 pending 的中断再等；
    - `Stop`：no-op 桩。
  - **已知实现局限**：Huffman 仅支持 bits ∈ {2,4,8}、尾部不足 4 字节未写回；BitUnPack 仅支持目标位宽 8/16/32。

## 4. 关键技术与"血泪教训"（改代码前必读）

1. **PC 流水线硬约定**（全项目最重要）：
   `regs[15]` 在执行指令期间**恒等于"读 PC 的硬件可见值"**：
   ARM = 指令地址 + 8；Thumb = 指令地址 + 4。
   取指地址 = `regs[15] - 8`（ARM）/ `- 4`（Thumb）。
   **分支落地必须写 `目标 + 8/+4`** 以维持该约定（`write_pc_arm` / `write_pc_thumb` 是唯一出口）。
   派生怪癖：`BL` 的 LR = 指令 + 4；ARM 态 `STR pc` 存"指令 + 12"；Thumb 的字对齐 PC 相对访问要 `& !2`。
2. **Thumb 分派表按位模式区间**，不是 GBATEK 的格式编号：
   `match insn >> 10 { 0..=1 => F1, 2..=7 => F2, 8..=15 => F3, 16 => F4, 17 => F5, 18..=19 => F6, 20..=23 => F7/F8, 24..=27 => F9, 32..=35 => F10, 36..=39 => F11, 40..=43 => F12, 44 => F13, 45..=47 => F14, 48..=51 => F15, 52..=54 => F16, 55 => F17, 56..=59 => F18, 60..=63 => F19 }`。
   28..=31 是保留区。**改这里前先跑测试**。
3. **`category_000` 的判别（位 7、4 同时为 1 的杂烩区）**：
   位 24 = 0 → 乘法；否则再看**位 6-5 两位**：`00` = SWP/SWPB；`01/10/11` = LDRH / LDRSB / LDRSH。
   **绝不能只判一位**（SWPB 的位 22 是字节标志、LDRH 的位 6 = 0——只判任一位都会误判，导致内存被静默改写）。
4. **块传输统一算法**：先算"最低地址 low"，然后**寄存器恒按编号升序从 low 向上排**（递减模式的"递减"只描述 low 怎么算，不改变映射方向）；写回按 GBATEK 的 ULD 规则（Rn 在列表中的最低编号位置决定）。
5. **错误策略**：合法但未实现 → `CpuError::Unimplemented`（`Err` 上报）；真机就是未定义的编码（协处理器等）→ `enter_exception(0x04, Undefined)`。两者语义不同，不要混。
6. **测试期望值必须手工演算**（用 `python3 -c` 算编码/偏移/标志）。历史上 8 个失败里有 6 个是测试自己写错——先怀疑测试，再怀疑实现；"真 bug"通常藏在**边界语义**（总线宽度、初始化一致性、位掩码）。
7. 总线怪癖（`bus.rs`）：调色板/VRAM 背景区 **8 位写会复制进整个半字**（OBJ 区不复制）；OAM **忽略 8 位写**；SRAM 宽读**字节重复**（`0x5A5A5A5A`）；镜像掩码规则；ROM 窗口跨 0x0A 段镜像偏移按 `(addr-0x08000000)&0x01FFFFFF`。

## 5. 验证与运行命令

```bash
cd D:\project\rust\gbars
cargo test                 # 59 通过（当前基线）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
# 不开窗口跑核心（M0 遗留：打印头部 + 输出 SMPTE 彩条 BMP）
cargo run -p gba-core --example headless -- "D:/Games/gba roms/Pokemon.gba"
```

## 6. 下一步任务清单（按顺序）

1. **（可选）ArmWrestler 冒烟验收**（先把 `examples/headless.rs` 加一个"跑 N 步"模式，跑完打印 PC/模式/步数）：
   `test-roms/armwrestler.gba` 已就位（destoer 修正版，纯汇编、零 SWI 依赖）。
   **其测试结果只通过屏幕文字输出**（源码 `DrawText` 例程；ROM 内无 no$gba 日志寄存器、无内存结果结构）。可选路线：
   - **（推荐）跳过 M1 内的严格验收，直接进入 M2**：M2 用 FireRed 进标题画面同时验证 CPU 与图形；
   - 最低限度冒烟：跑 ~200 万步不 panic、PC 不落入未映射区、无 `Unimplemented` 错误、不卡死在同一地址。
2. **进入 M2**：统一调度器（16.777216 MHz 主频、280,896 周期/帧）→ PPU（模式 0–5、精灵、窗口、混合）→ 帧输出接 `frame.rs` 契约 → 验收"**FireRed 进标题画面且渲染正确**"。
   届时把 `cpu` 的周期计量补上（现在只有指令条数 `steps`，无周期模型）。
3. 按需更新 `README.md` 的里程碑进度与本文件。

> 注意：**不要再写 `docs/notes/` 文档、不要加代码注释**（新约定见 §1）。

## 7. 未完成/已知限制总表

| 项 | 状态 | 解决时机 |
|---|---|---|
| IntrWait/VBlankIntrWait 真语义（挂起+唤醒） | 桩（立即返回） | M3 中断控制器 |
| Halt 唤醒 | 桩（永久休眠标志） | M3 |
| 周期级时序（等待状态/预取） | 无（只有 `steps` 计数） | M2 调度器 |
| Huffman 位宽 {2,4,8} 外的形态、尾部字节 | 保守放弃 | 按需（FireRed 未见） |
| BitUnPack 目标位宽 <8 的打包写 | 未实现 | 按需 |
| GPIO/RTC、EEPROM 存档、SRAM/Flash 存档 | 未开始 | M3 |
| PPU/APU/DMA/定时器/中断/按键 | 未开始 | M2–M4 |

## 8. 参考资源

- GBATEK（problemkaputt.de/gbatek.htm）：一切硬件行为的最终依据；
- 本地 mGBA 源码 `D:\project\rust\mgba`（主题对应文件见 design.md §3.2）；
- 测试 ROM：`test-roms/armwrestler.gba`（已下载）；gba-tests 系列可后续从 `destoer/gba-tests`（GitHub）获取；
- 历史学习文档：`docs/notes/00 → 01 → 02`（仅作背景参考，不再新增）。