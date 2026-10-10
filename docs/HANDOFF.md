# 交接文档（HANDOFF）

> 写给接手的 AI 或开发者：读完这一篇即可无缝继续。
> 最后更新：2026-10-09。

## 0. 一句话现状

GBA 模拟器项目 **gbars** 位于 `D:\project\rust\gbars\`。
**M0（骨架）与 M1 第一、二步（全部 ARM/Thumb 指令）已完成；
M1 第三步（SWI HLE）刚完成并通过全部测试（59 通过，fmt/clippy 零警告）。
当前所处位置：M1 收尾 —— 下一步是"跑 ArmWrestler 验收 + 写 notes/03"，然后进入 M2（PPU 图形）。**

## 1. 项目背景与约定（必须遵守）

| 项 | 内容 |
|---|---|
| 目标 | 用 Rust 写 GBA 模拟器（核心纯库）+ Tauri 2 + React + MUI 前端，最终完整运行 `D:\Games\gba roms\Pokemon.gba`（**Pokemon FireRed 美版 v1.1**，BPRE，存档类型 Flash 128KB/`FLASH1M_V103`，无 RTC） |
| 设计方案 | `docs/design.md`（权威版；架构、IPC 协议、M0–M6 里程碑表） |
| **学习约定（用户硬性要求）** | ① 全部代码用**中文教学注释**：模块头 `//!` 讲硬件背景与参考资料，行内注释讲"为什么这样设计/模拟"；② 每个里程碑/关键子步在 `docs/notes/NN-主题.md` 产出一份说明文档，**固定五段结构**：做了什么 / 背景知识 / 代码走读 / 如何验证 / 下一步预告 |
| 工程基线（每次交付必须全绿） | `cargo fmt --check`；`cargo test`（当前 59 个）；`cargo clippy --workspace --all-targets -- -D warnings` |
| 工具链 | rustc/cargo 1.99.0、edition 2024、node v24、pnpm 12 |
| mGBA 源码 | 已克隆在 `D:\project\rust\mgba`（实现时按主题对照：`src/arm/` CPU、`src/gba/bios.c` SWI HLE、`src/gba/memory.c` 总线、`src/gba/hle-bios.s` 引导状态） |
| **环境坑** | 本环境有安全钩子：**禁止用 bash（sed/echo 等）直接写源码文件**，必须用 Write/Edit 工具；git 已初始化（main 分支）**但从未提交**，接手后可先做一次初始提交 |

## 2. 目录与文件地图

```text
gbars/
├── Cargo.toml                       # workspace（release: lto+codegen-units=1）
├── README.md                        # 里程碑进度 + notes 索引
├── docs/
│   ├── design.md                    # 设计方案（权威）
│   ├── HANDOFF.md                   # ← 本文
│   └── notes/
│       ├── 00-project-skeleton.md   # M0：头部解析/校验和演算/BMP
│       ├── 01-memory-bus-and-arm-core.md   # 总线怪癖/银行/移位器/调试实录
│       └── 02-block-transfers-and-thumb.md # LDM/STM/Thumb 全集/调试实录
├── test-roms/armwrestler.gba        # 已下载的 CPU 测试 ROM（16,936 B，非商业）
└── crates/gba-core/src/
    ├── lib.rs                       # 门面 + 模块地图
    ├── cartridge.rs                 # ROM 装载与头部解析（M0）
    ├── frame.rs                     # 帧契约 240×160 RGBA8888（M0）
    ├── bus.rs                       # 内存总线（M1）
    ├── gba.rs                       # GameBoyAdvance 门面 + 集成测试（M1）
    ├── bios.rs                      # ★ SWI 高层模拟（M1 第三步，刚完成）
    └── cpu/
        ├── mod.rs                   # 模式/银行/异常/PC 约定/step()
        ├── alu.rs                   # 桶形移位器与标志位
        ├── isa_arm.rs               # ARM 全指令
        └── isa_thumb.rs             # Thumb 全 19 格式
```

## 3. 刚完成的工作（M1 第三步，本次会话）

`bios.rs` 从零实现并经 mGBA 对照验证，**已接入 ARM 与 Thumb 两条 SWI 路径**：

- **已实现（HLE 直接算结果，PC 像普通指令推进，调用者无感知）**：
  `SoftReset`、`RegisterRamReset`、`Div/DivArm`、`Sqrt`、`ArcTan/ArcTan2`、
  `CpuSet/CpuFastSet`、`GetBiosChecksum`、`BgAffineSet/ObjAffineSet`、
  `BitUnPack`、`LZ77 解压（Wram/Vram 两变体）`、`RL 解压`、`Huffman 解压`、
  `MidiKey2Freq`；Sound driver 系列（0x16–0x1E）按 no-op。
- **有意留桩（等后续里程碑）**：
  - `Halt`：置 `cpu.halted = true`（`step()` 直接返回）。**唤醒逻辑要等 M3 中断控制器**；
  - `IntrWait/VBlankIntrWait`：**目前立即返回（no-op）**。M3 需改成"清 IF → 挂起 → 中断唤醒"真语义——官方 BIOS 的 IntrWait 还会先吐掉已经 pending 的那个中断再等，M3 实现时注意；
  - `Stop`：no-op 桩。
- **SWI 语义变更注意**：SWI 不再触发异常入口（不再进 Supervisor 模式）——旧的 `swi_enters_supervisor` 测试已改写为 `swi_div_is_hle`。异常返回路径的测试保留（改用 `enter_exception` 手工构造）。
- **已知实现局限（代码内已注明）**：Huffman 仅支持 bits ∈ {2,4,8}、尾部不足 4 字节未写回；BitUnPack 仅支持目标位宽 8/16/32。

## 4. 关键技术与"血泪教训"（改代码前必读）

1. **PC 流水线硬约定**（全项目最重要）：
   `regs[15]` 在执行指令期间**恒等于"读 PC 的硬件可见值"**：
   ARM = 指令地址 + 8；Thumb = 指令地址 + 4。
   取指地址 = `regs[15] - 8`（ARM）/ `- 4`（Thumb）。
   **分支落地必须写 `目标 + 8/+4`** 以维持该约定（`write_pc_arm` / `write_pc_thumb` 是唯一出口）。
   派生怪癖：`BL` 的 LR = 指令 + 4；ARM 态 `STR pc` 存"指令 + 12"；Thumb 的字对齐 PC 相对访问要 `& !2`。
2. **Thumb 分派表按位模式区间**，不是 GBATEK 的格式编号：
   `match insn >> 10 { 0..=1 => F1, 2..=7 => F2, 8..=15 => F3, 16 => F4, 17 => F5, 18..=19 => F6, 20..=23 => F7/F8, 24..=27 => F9, 32..=35 => F10, 36..=39 => F11, 40..=43 => F12, 44 => F13, 45..=47 => F14, 48..=51 => F15, 52..=54 => F16, 55 => F17, 56..=59 => F18, 60..=63 => F19 }`。
   28..=31 是保留区。**改这里前先跑测试**——历史上就是这里翻过车（详解见 notes/02）。
3. **`category_000` 的判别（位 7、4 同时为 1 的杂烩区）**：
   位 24 = 0 → 乘法；否则再看**位 6-5 两位**：`00` = SWP/SWPB；`01/10/11` = LDRH / LDRSB / LDRSH。
   **绝不能只判一位**（SWPB 的位 22 是字节标志、LDRH 的位 6 = 0——只判任一位都会误判，导致内存被静默改写）。
4. **块传输统一算法**：先算"最低地址 low"，然后**寄存器恒按编号升序从 low 向上排**（递减模式的"递减"只描述 low 怎么算，不改变映射方向！）；写回按 GBATEK 的 ULD 规则（Rn 在列表中的最低编号位置决定），已实现于 `block_transfer` 尾部。
5. **错误策略**：合法但未实现 → `CpuError::Unimplemented`（`Err` 上报）；真机就是未定义的编码（协处理器等）→ `enter_exception(0x04, Undefined)`。两者语义不同，不要混。
6. **测试期望值必须手工演算**（用 `python3 -c` 算编码/偏移/标志）。历史上 8 个失败里有 6 个是测试自己写错——先怀疑测试，再怀疑实现；但"真 bug"通常藏在**边界语义**（总线宽度、初始化一致性、位掩码）。
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

1. **headless 加"跑 N 步"模式**：`examples/headless.rs` 增加可选的 step 数参数，跑完打印 CPU 状态（PC/模式/最近执行的指令数），用于 ArmWrestler 冒烟测试；
2. **ArmWrestler 冒烟运行**：`test-roms/armwrestler.gba` 已就位（destoer 修正版，纯汇编、零 SWI 依赖）。
   **已确认：它的结果只通过屏幕文字输出**（源码 `DrawText` 例程；ROM 内无 no$gba 日志寄存器引用、无内存结果结构）。可选路线：
   - **(推荐) 直接进入 M2 做 PPU**，M2 验收时用 ArmWrestler 屏幕文字同时验证 CPU 与图形（设计中 M2 验收本来就要"FireRed 进标题画面"）；
   - 若坚持 M1 内验收：可在 headless 里 dump VRAM 的 tilemap 并用 ROM 内字体表反解文字（工作量中等，收益低）；
   - **最低限度冒烟**：跑 200 万步不 panic、PC 不落入未映射区、无 `Unimplemented` 错误、不卡死在同一地址；
3. **写 `docs/notes/03-swi-hle.md`**（五段结构；素材：本篇 §3 的 SWI 清单、LZ77/RL 位流格式、GBATEK 对照点；可讲"为什么 HLE 不用真 BIOS"与 "SWI 从异常入口改成 HLE 的语义变化"）；
4. **更新 README 与记忆**（README 的 M1 行改为"SWI HLE 完成"）；
5. **进入 M2**：统一调度器（16.777216 MHz 主频、280,896 周期/帧）→ PPU（模式 0–5、精灵、窗口、混合）→ 帧输出接 `frame.rs` 契约 → M2 验收"FireRed 进标题画面"。
   届时更新 `cpu` 的周期计量（现在只有指令条数 `steps`，无周期模型）。

## 7. 未完成/已知限制总表

| 项 | 状态 | 解决时机 |
|---|---|---|
| IntrWait/VBlankIntrWait 真语议（挂起+唤醒） | 桩（立即返回） | M3 中断控制器 |
| Halt 唤醒 | 桩（永久休眠标志） | M3 |
| 周期级时序（等待状态/预取） | 无（只有 `steps` 计数） | M2 调度器 |
| Huffman 位宽 {2,4,8} 外的形态、尾部字节 | 保守放弃 | 按需（FireRed 未见） |
| BitUnPack 目标位宽 <8 的打包写 | 未实现 | 按需 |
| GPIO/RTC、EEPROM 存档、SRAM/Flash 存档 | 未开始 | M3 |
| PPU/APU/DMA/定时器/中断/按键 | 未开始 | M2–M4 |
| git 提交 | **仓库从未提交过** | 接手第一件事建议提交 |

## 8. 参考资源

- GBATEK（problemkaputt.de/gbatek.htm）：一切硬件行为的最终依据；
- 本地 mGBA 源码 `D:\project\rust\mgba`（CPU/SWI/总线/存档/PPU 各主题对应文件见 design.md §3.2）；
- 测试 ROM：`test-roms/armwrestler.gba`（已下载）；gba-tests 系列可后续从 `destoer/gba-tests`（GitHub）获取；
- 学习文档阅读顺序：`docs/notes/00 → 01 → 02 →（03 撰写中）`。
