# gbars

> 用 Rust 从零实现的 Game Boy Advance 模拟器。
>
> 技术栈：Rust（模拟核心）· Tauri 2（桌面壳）· React + MUI（界面）
> 验收目标：完整运行《Pokemon FireRed》

## 代码与文档约定

**本项目不写代码注释、不写解释文档**（约定见 [docs/design.md](docs/design.md) §1.4，2026-10-10 起生效）。
`docs/notes/` 下的文档为历史存档，不再新增；现有注释与文档原样保留。

| # | 文档 | 内容 |
|---|------|------|
| — | [docs/design.md](docs/design.md) | 总体设计方案：架构、模块、IPC 协议、里程碑（权威） |
| — | [docs/HANDOFF.md](docs/HANDOFF.md) | 交接文档：现状、约定、下一步任务 |
| 00 | [docs/notes/00-project-skeleton.md](docs/notes/00-project-skeleton.md) | 项目骨架：workspace、ROM 头部解析、帧契约 |
| 01 | [docs/notes/01-memory-bus-and-arm-core.md](docs/notes/01-memory-bus-and-arm-core.md) | 内存总线、ARM 核心（历史） |
| 02 | [docs/notes/02-block-transfers-and-thumb.md](docs/notes/02-block-transfers-and-thumb.md) | 块传输与 Thumb（历史） |

## 里程碑进度

- [x] **M0 骨架** —— workspace、卡带装载与头部解析、帧契约、headless CLI
- [ ] **M1 CPU**（收尾中）—— ARM + Thumb 指令集与 SWI HLE 全部落地；剩余：ArmWrestler 验收。详见 [docs/HANDOFF.md](docs/HANDOFF.md)
- [ ] **M2 图形** —— 统一调度器、PPU 全模式渲染
- [ ] **M3 系统** —— 中断 / 定时器 / DMA、Flash 128KB 存档
- [ ] **M4 音频** —— PSG 四通道 + DirectSound FIFO
- [ ] **M5 前端整合** —— Tauri 线程模型 + IPC + MUI 界面
- [ ] **M6 打磨** —— 快进、即时存档、长时间稳定性验证

## 常用命令

```bash
cargo build                                            # 构建
cargo test                                             # 运行全部测试
cargo fmt --check                                      # 格式检查
cargo clippy --workspace --all-targets -- -D warnings  # 静态检查（零警告基线）

# 无窗口运行核心（调试/回归入口）
cargo run -p gba-core --example headless -- "D:/Games/gba roms/Pokemon.gba"
```

## 目录结构

```text
crates/gba-core/   # 模拟核心库（禁止依赖 UI 库；M5 的 Tauri 应用将以新成员加入）
docs/design.md     # 设计方案（仓库内为权威版本）
docs/HANDOFF.md    # 交接文档
docs/notes/        # 历史学习文档（不再新增）
```

## 法律说明

仓库不附带任何 ROM、BIOS 或 Nintendo 版权数据（包括开机 logo）。
ROM 由使用者自行提供；本模拟器通过高层模拟（HLE）实现无 BIOS 启动。
