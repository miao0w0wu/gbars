# 交接文档（HANDOFF）

> 写给接手的 AI 或开发者：读完这一篇即可无缝继续。
> 最后更新：2026-10-11（M2 收尾中：修复 6 处真 bug，FireRed 引导深入到 IntrMain 每帧分发 VBlank IRQ；卡点 = 引导期初始化数据仍有约 94 字节级偏差，handler 表项为 0）。

## 0. 一句话现状

GBA 模拟器项目 **gbars** 位于 `D:\project\rust\gbars\`。
**M0/M1 已提交；M2 图形主体在 `9078372`；九处 bug 修复在 `22148de`；本次会话六处 bug 修复在 `e3d4a71`。**
**`cargo test` 91 全绿、clippy `-D warnings` 与 fmt 全绿。**

- ArmWrestler 菜单画面渲染正确（无回归）；
- FireRed 引导推进到 **IntrMain 正常运行**：每帧一次 VBlank IRQ、handler 分发循环执行，
  但 IntrMain 分发到 **空的 handler 表项**（游戏初始化写入的数据仍有少量字节级偏差），
  `BX 0` 后滑过 BIOS 零区回到 IRQ 桩，形成每帧循环，DISPCNT 停在 0x0080。

工作区干净，全部已提交。HEAD = `e3d4a71`。

## 1. 项目背景与约定（必须遵守）

| 项 | 内容 |
|---|---|
| 目标 | 用 Rust 写 GBA 模拟器（核心纯库）+ Tauri 2 + React + MUI 前端，最终完整运行 `D:\Games\gba roms\Pokemon.gba`（**Pokemon FireRed 美版 v1.1**，BPRE，存档类型 Flash 128KB/`FLASH1M_V103`，无 RTC） |
| 设计方案 | `docs/design.md`（权威版；架构、IPC 协议、M0–M6 里程碑表） |
| **代码/文档约定（用户硬性要求，2026-10-10 起）** | **① 不写任何代码注释（含模块头 `//!` 与行内注释）；② 不写解释文档，不再产出 `docs/notes/`。** 既有注释/文档保留不动。正确性由测试与 FireRed 验收保证 |
| 工程基线（每次交付必须全绿） | `cargo fmt --check`；`cargo test`（当前 91 个）；`cargo clippy --workspace --all-targets -- -D warnings` |
| 工具链 | rustc/cargo 1.99.0、edition 2024、node v24、pnpm 12 |
| mGBA 源码 | `D:\project\rust\mgba`（**本次已建 headless 对拍环境，见 §5**） |
| **环境坑 1** | 安全钩子**禁止用 bash（sed/echo 等）直接写源码文件**，必须用 Write/Edit 工具 |
| **环境坑 2** | Git Bash 里 `cargo run ... \| head` 偶发吞 stdout——重定向到文件再 grep 可靠；shell 的 cwd 会停留在最后一次 `cd` 的目录，每个命令显式 `cd /d/project/rust/gbars` |

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
    ├── bus.rs                       # 内存总线 + IRQ 桩 + last_fetch（开总线）
    ├── gba.rs                       # GameBoyAdvance 门面 + run_frame()
    ├── bios.rs                      # SWI HLE（IntrWait 挂起/唤醒）
    ├── scheduler.rs                 # 统一时钟；run_frame/run_instruction/now()
    ├── io.rs                        # 0x04000000 寄存器层（DMA 正确地址映射）
    ├── timer.rs                     # 4×16 位定时器
    ├── dma.rs                       # 4 通道 DMA
    ├── ppu/                         # mod/background/bitmap/sprite/blending/window
    └── cpu/
        ├── mod.rs                   # 模式/银行/异常/step()/cond_pass
        ├── alu.rs                   # 桶形移位器与标志位
        ├── isa_arm.rs               # ARM 全指令
        └── isa_thumb.rs             # Thumb 全 19 格式
```

## 3. 本次会话完成的工作（e3d4a71）

### 3.1 未映射 IO 的开总线（FireRed 随机种子循环的真实解法）

- 交接文档旧方案（"写 0x83 到 0x118 读回"）**与真实代码不符**：FireRed 的种子例程
  （0x081DF184 起）把 0x83 写到 **0x0400011E**，然后轮询 **0x04000118**——两个不同地址。
  真机/mGBA 上未映射 IO 的读返回**总线残值**，不是写入值；
- 实现：`Bus.last_fetch: u16`（每次取指更新，`fetch_u32` 取高半字），
  `Io::read_u16(addr, open_bus)` 未命中专用读路径时返回它（对照 mGBA `GBALoadBad`）；
  原始 u16 暂存数组保留，仅用于 8 位写的读-改-写合并（mGBA io[] 合并语义）；
  `Ppu::read_reg` 改返回 `Option<u16>`，`None` = 不属于 PPU、走开总线。

### 3.2 六处真 bug（按因果链排序）

| # | 文件 | 缺陷 | 后果 |
|---|---|---|---|
| 1 | `io.rs` DMA 映射 | **DMA 寄存器地址整体 +8**：把 CNT 当成 SAD。真实布局（mGBA io.h = 真机）每通道 12 字节：SAD+0/DAD+4/CNT+8，DMA0 始于 0xB0 | FireRed 的 IntrMain 拷贝（DMA3：SAD=0x08000248→DAD=0x03003580，CNT 字写 0x84000200）全部落错通道，IWRAM 里 handler 代码是零 |
| 2 | `isa_arm.rs` 分发 | 位 27-25 = `0b011` 是**寄存器偏移 LDR/STR**，被当成协处理器空间直接进 Undefined 异常 | IntrMain 里 `STRB r0,[r5,r6]`（0xE7C50006）触发异常 → CPU 出轨 |
| 3 | `isa_thumb.rs` F13 | 组 44（0xB000-0xB3FF）全部路由给 F12 处理器；F13（ADD/SUB SP,#imm7×4，位 7=加/减）有独立编码 | **每条 SUB SP 都污染一个低寄存器**（执行成 `ADD Rd,PC,#imm8`）——0xB081 使 r0=0x081DD718 |
| 4 | `isa_thumb.rs` SWI 组 | 组 55（0xDC00-0xDFFF）= 条件分支 BGT/BLE/AL + SWI(0xDF00)；旧代码把整组当 SWI | 0xDCF8（BGT）执行成 SWI 0xF8，36 次循环 1 次就退出 |
| 5 | `isa_thumb.rs` F9 | 子操作码表**互换**：真实是 00=STR 01=LDR 10=STRB 11=LDRB，旧码 01=STRB 10=LDR | **所有 Thumb LDR 变成字节存储（破坏内存），所有 STRB 变成字加载**——IWRAM 初始化数据大面积损坏的根源 |
| 6 | `dma.rs`/`scheduler.rs` 测试 | 测试里的 DMA 寄存器地址沿用旧错误映射 | 已按正确布局重写（#1 的配套） |

### 3.3 mGBA headless 对拍环境（本次最大方法论收获）

`D:\project\rust\mgba` 里新增：
- `harness2.c`：基于 perf-main.c 骨架的单步挂钩——`core->step` = 恰好一条指令，
  可按 PC 设观察点、dump `gba->memory.dma[i]`、全量 fwrite IWRAM+EWRAM 快照；
- `src/platform/test/CMakeLists.txt` 末尾加了 `mgba-harness` 目标（CMAKE_SOURCE_DIR/harness2.c）；
- 构建命令与坑见 §5。**注意 mGBA 结构体陷阱：`gba->memory.iwram` 是 `uint32_t*`**，
  按字节访问要转 `(uint8_t*)`。

对拍三板斧（本次实战验证有效，按顺序用）：
1. **SWI 序列对比**：两边记录 `SWI 0B/0C` 的 (r0,r1,r2,lr)。mGBA 侧检测 insn 要排除
   它 HLE BIOS 里的内部指令 `swi 0xF00000`（出现在 BIOS 0x78，是误报源）；
2. **PC 轨迹对比**：从某同步点（如 CpuSet 返回地址 0x081DD594）各录 N 条 PC，diff 找分叉；
3. **内存快照对比**：最后一个共同 PC 处（如 0x081E40A6）各 dump IWRAM 32K + EWRAM 256K，
   按字节 diff 出差异段。本次用它把 335 字节差异收敛到 94 字节。

### 3.4 FireRed 引导现状（0x08/0x18 级事实，改代码前必读）

- **crt0 入口 0x08000204 自己设置栈**：`sp_sys = 0x03007E40`（不是 BIOS 默认 0x7F00！）、
  `sp_svc = 0x03007FA0`、IntrVector 先指向 ROM 里的 IntrMain 0x08000248；
- IntrMain 拷贝用 **DMA3 立即传输**（0x080006B2 起：SAD=0x08000248、DAD=0x03003580、
  CNT 字 = 0x84000200 = 0x200 半字 + enable），然后 0x080006C0 把 0x03003580 写入
  0x03FFFFFC——我们的 DMA 修好后这一步已正常；
- 游戏清 IWRAM 0x00-0x5F、把 0x60-0xBF 填 0xFF（0x08000990-0x9A2 的 STRB 循环，
  F9 修复后已正常）；CpuSet 序列 #1/#2/#3 与 mGBA 逐参数一致；
- **当前卡点**：IntrMain（0x03003580 起，ARM）每帧被 VBlank IRQ 唤醒，分发循环跑到
  handler 表项 = 0（游戏注册的中断处理表仍有数据偏差），`BX 0` → BIOS 零区滑行 →
  0x18 IRQ 桩 → IntrMain → 死循环。DISPCNT 停在 0x0080；
- 同步点 0x081E40A6 处内存快照还剩 **94 字节差异**，集中在：
  - IWRAM 0x5F50-0x5F81：CpuSet#2 目标块（源 = 栈 0x7E08 环绕区，栈上构造的表仍有零值）；
  - IWRAM 0x7010/0x7060：函数指针表项——mGBA 是各不相同的 ROM 指针
    （0x081DE39D/0x081DE4F5），我们重复填了同一个 0x081DC851；
  - IWRAM 0x7081 起 0x24 步长的 7 条记录各差 1 字节（CpuSet#3 目标 0x03007080 的数据）；
  - 栈 0x7DE8-0x7DF9：mGBA 有 0x04000006/0x030030F0/0x00040000 等局部值，我们是零。

## 4. 关键技术与"血泪教训"（改代码前必读）

1. **PC 流水线硬约定**：`regs[15]` 执行期 = 指令地址 + 8（ARM）/+ 4（Thumb）；
   取指地址 = `regs[15] - 8/-4`；分支落地写 `目标 + 8/+4`（`write_pc_arm/thumb` 唯一出口）。
2. **Thumb 译码按 `insn >> 10` 分组**，组边界易错：44 = 0xB0xx（F13，不是 F12！）、
   45..=47 = PUSH/POP、52..=55 = 条件分支+SWI（cond=0xF 才是 SWI）。**改译码表必配单测**。
3. **F9 子操作码：00=STR 01=LDR 10=STRB 11=LDRB**（位 12=字节、位 11=加载）。
   F10 半字用位 11=load；F11 SP 相对同。写内存指令的测试要断言"目标内存 + 寄存器"双侧。
4. **DMA 寄存器布局（改 io.rs 前必查）**：DMA0 SAD=0xB0；每通道 12 字节 SAD+0/DAD+4/CNT+8；
   CNT_H 位域以 mGBA `DECL_BITS(DestControl,5,2)/DECL_BITS(SrcControl,7,2)` 为准
   （dst=位5-6、src=位7-8、repeat=9、width=10、timing=12-13、irq=14、enable=15）。
5. **开总线模型**：未映射 IO 读 = `Bus.last_fetch`（最近取指半字）；写 = 落 io 暂存数组
   （仅用于 8 位写合并）。别再回到"写读回"模型——FireRed 种子例程读写不同地址。
6. **测试期望值必须手工演算**（本次又错 4 处：位段写错、字节替换算错、STR 的 Rt 位）。
   0x0400_0118 这类字面量必须 8 个十六进制位（下划线按 4 位一组数）。
7. **mGBA 对拍挂钩的读取不扰动模拟时序**（`busRead16` 不计周期），我们自己的探针
   `bus.read_u16` 会 +1 周期/步拖慢时钟 ~30%，**分叉点会漂移**——先看现象别信绝对位置。
8. **SWI 检测要排除 mGBA 内部指令**：`swi 0xF00000`（hle-bios 的 stall 机制）在 BIOS 0x78。
9. **排查方法论（升级版）**：SWI 序列 → PC 轨迹 → 内存快照，三层递进；同步点选
   "最后一个共同 PC"；mGBA 结构体字段注意指针/数组（iwram 是 uint32_t*）。
10. **总线怪癖**（bus.rs）：调色板/VRAM 背景区 8 位写复制进整个半字（OBJ 区不复制）；
    OAM 忽略 8 位写；SRAM 宽读字节重复；ROM 窗口外读 0。
11. **IRQ 桩**：BIOS 0x18 = `b 0x120`；irqBase 保存 {r0-r3,r12,lr} → LDR pc,[0x03FFFFFC]
    → LDMFD → SUBS pc,lr,#4。IntrMain 内部再跳 0x130 属正常流程，不是 bug。
12. **SWI HLE**：IntrWait 清 IF + halted；`service_irq` 唤醒。未知 SWI no-op。

## 5. 验证与运行命令

```bash
cd D:\project\rust\gbars
cargo test                 # 91 通过（当前基线）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# ArmWrestler 600 帧 → frame.bmp（菜单文字正常）
cargo run -p gba-core --example headless -- test-roms/armwrestler.gba 600 frame.bmp

# FireRed 300 帧 → firered.bmp（当前卡在 IntrMain，DISPCNT=0x0080）
cargo run -p gba-core --example headless -- "D:/Games/gba roms/Pokemon.gba" 300 firered.bmp
```

mGBA 对拍环境（一次性构建好了，产物在 `build-harness/`）：

```bash
cd D:\project\rust\mgba
cmake -B build-harness -G Ninja -DBUILD_QT=OFF -DBUILD_SDL=OFF -DBUILD_PERF=ON \
  -DUSE_FFMPEG=OFF -DUSE_MINIZIP=OFF -DUSE_PNG=OFF -DUSE_LIBZIP=OFF -DUSE_ELF=OFF \
  -DUSE_DISCORD_RPC=OFF -DUSE_SQLITE3=OFF -DUSE_GDB_STUB=OFF -DBUILD_SUITE=OFF \
  -DBUILD_EXAMPLE=OFF -DUSE_EPOXY=OFF -DLIBMGBA_ONLY=ON -DBUILD_STATIC=ON \
  -DBUILD_GL=OFF -DBUILD_GLES2=OFF -DBUILD_GLES3=OFF
cmake --build build-harness        # 产物 build-harness/mgba-harness.exe 与 mgba-perf.exe
./build-harness/mgba-perf.exe -F 60 "D:/Games/gba roms/Pokemon.gba"   # 冒烟（约 3000fps）

# 链接坑：harness 必须与 LTO 静态库同为 -flto，否则 ld 静默崩溃（exit 5，无任何输出）
# 头文件坑：mCoreFind/mCoreLoadFile 需要 -DENABLE_VFS -DENABLE_VFS_FD
# 链接库集合照抄 build.ninja 里 mgba-perf 的 LINK_LIBRARIES
```

改 `harness2.c` 后：`cmake --build build-harness` 即可（已在 test/CMakeLists.txt 注册
`mgba-harness` 目标）。观察点写法参考 harness2.c 现有代码（0x081DD510 入口、
0x081DF184 入口、0x081DF1A0 DMA 触发、内存快照 dump）。

## 6. 下一步任务清单（按顺序）

1. **继续收敛 94 字节差异**：对 0x5F50 块 / 0x7010 函数指针表 / 0x7081 记录，向上追
   "谁写的"（mGBA 挂钩里对这两个地址做写监视，记录首个写入 PC，与我们对比）。
   疑点集中在 CpuSet#3 源（栈 0x7E04 构造区）与 m4a 引擎初始化对 [SoundInfo+0x14] 的写。
2. handler 表修好后 IntrMain 应完成分发 → IntrWait 主循环 → DISPCNT 脱离 0x0080 →
   300 帧内出标题画面（M2 验收）。
3. **M3 系统**：中断控制器完善、KEYINPUT（现恒 0x3FF）、Flash 128KB 存档。
   验收 = FireRed 新游戏 → 走入世界 → 存档重启可 CONTINUE。
4. 按需更新 `README.md` 里程碑与本文件；**继续遵守"不写注释/不写解释文档"约定**。

## 7. 未完成/已知限制总表

| 项 | 状态 | 解决时机 |
|---|---|---|
| FireRed 引导数据级偏差（94 字节，handler 表项为 0） | **当前卡点** | 立即（§6.1） |
| HBlank 精确时机（960/272 细分）、HBlank DMA 时序 | 整行近似 | M3/M6 按需 |
| DMA 触发方式 3（声音 FIFO/视频捕获） | 未实现 | M4 音频 |
| GPIO/RTC、EEPROM、SRAM/Flash 存档 | 未开始 | M3 |
| APU 音频 | 未开始 | M4 |
| 按键输入（KEYINPUT 恒 0x3FF 全未按） | 未开始 | M3 |
| Huffman 仅 bits ∈ {2,4,8}、BitUnPack 位宽 8/16/32 | M1 遗留限制 | 按需 |
| mGBA 同帧像素对拍（截图级） | 未做（状态级对拍已通） | M2 收尾 |

## 8. 参考资源

- GBATEK（problemkaputt.de/gbatek.htm）：硬件行为最终依据（直连与 WebFetch 都 403，
  用 mGBA 源码宏定义交叉验证，本次 DMA 位域/寄存器地址即如此定案）；
- 本地 mGBA 源码 `D:\project\rust\mgba`：`io.h`（**寄存器地址唯一权威**）、
  `dma.h`（位域 DECL_BITS）、`dma.c`（GBADMAWriteCNT_HI 触发语义）、
  `memory.c`（GBALoadBad 开总线）、`hle-bios.s`（IRQ 桩 + SWI 汇编实现）、
  `bios.c`（RegisterRamReset 语义）；
- **mGBA 对拍挂钩**：`mgba/harness2.c`（用法见 §5）；
- 测试 ROM：`test-roms/armwrestler.gba`；gba-tests 系列可从 `destoer/gba-tests`（GitHub）获取；
- 项目约定：不写解释文档、不写代码注释（历史遗留注释保留不动）。
