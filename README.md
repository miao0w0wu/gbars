# gbars

> 用 Rust 从零实现的 Game Boy Advance 模拟器。
>
> 技术栈：Rust（模拟核心）· Tauri 2（桌面壳）· React + MUI（界面）
> 验收目标：完整运行《Pokemon FireRed》

## 代码与文档约定

**本项目不写代码注释、不写解释文档**（约定见 [docs/design.md](docs/design.md) §1.4，2026-10-10 起生效）。

| # | 文档 | 内容 |
|---|------|------|
| — | [docs/design.md](docs/design.md) | 总体设计方案：架构、模块、IPC 协议、里程碑（权威） |
| — | [docs/HANDOFF.md](docs/HANDOFF.md) | 交接文档：现状、约定、下一步任务 |

## 里程碑进度

- [x] **M0 骨架** —— workspace、卡带装载与头部解析、帧契约、headless CLI
- [x] **M1 CPU** —— ARM + Thumb 指令集与 SWI HLE 全部落地；59 测试全绿。详见 [docs/HANDOFF.md](docs/HANDOFF.md)
- [ ] **M2 图形**（进行中）—— 调度器、PPU 全模式渲染、定时器、DMA、中断投递主体已落地（工作区待提交）；剩余：修复 clippy/fmt 基线、PPU 渲染输出为纯色待排查、M2 模块补测试、FireRed 标题画面验收。详见 [docs/HANDOFF.md](docs/HANDOFF.md)
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

# 无窗口运行核心：跑 N 帧输出 BMP + 状态摘要（调试/回归入口）
cargo run -p gba-core --example headless -- test-roms/armwrestler.gba 600 frame.bmp

# 单步跟踪前 N 条指令（调试用）
cargo run -p gba-core --example headless -- test-roms/armwrestler.gba --trace 64
```

## 目录结构

```text
crates/gba-core/   # 模拟核心库（禁止依赖 UI 库；M5 的 Tauri 应用将以新成员加入）
docs/design.md     # 设计方案（仓库内为权威版本）
docs/HANDOFF.md    # 交接文档
```

## 法律说明

仓库不附带任何 ROM、BIOS 或 Nintendo 版权数据（包括开机 logo）。
ROM 由使用者自行提供；本模拟器通过高层模拟（HLE）实现无 BIOS 启动。
