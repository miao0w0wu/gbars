# gbars

> 用 Rust 从零实现的 Game Boy Advance 模拟器 —— 一个边做边学的项目。
>
> 技术栈：Rust（模拟核心）· Tauri 2（桌面壳）· React + MUI（界面）
> 验收目标：完整运行《Pokemon FireRed》

## 这个仓库怎么"读"

本项目为学习而写，遵循两条约定（见 [docs/design.md](docs/design.md) §1.4）：

1. **教学型注释**：代码注释用中文，重点解释"为什么这样设计/模拟"，而非复述代码行为；
2. **每步一份说明文档**：每完成一个里程碑（或关键子步），`docs/notes/` 里会多一份说明文档，按编号顺序阅读即可跟着项目一起学。

| # | 文档 | 内容 |
|---|------|------|
| — | [docs/design.md](docs/design.md) | 总体设计方案：架构、模块、IPC 协议、里程碑 |
| 00 | [docs/notes/00-project-skeleton.md](docs/notes/00-project-skeleton.md) | 项目骨架：workspace、ROM 头部解析、帧契约 |

## 里程碑进度

- [x] **M0 骨架** —— workspace、卡带装载与头部解析、帧契约、headless CLI
- [ ] **M1 CPU** —— ARM7TDMI 解释器（ARM + Thumb）、无 BIOS 启动、SWI HLE
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
docs/notes/        # 每步的学习说明文档
```

## 法律说明

仓库不附带任何 ROM、BIOS 或 Nintendo 版权数据（包括开机 logo）。
ROM 由使用者自行提供；本模拟器通过高层模拟（HLE）实现无 BIOS 启动。
