# EXEC-M2：M2 收尾执行文档（临时，完成后删除）

目标：FireRed 引导数据收敛 → 出标题画面（M2 验收）→ mGBA 同帧像素对拍。
方法：mGBA/gbars 双侧 IWRAM 写监视对拍，定位 94 字节偏差的首个分叉写。

## 步骤

| # | 任务 | 状态 |
|---|---|---|
| S1 | mGBA harness2.c 加 IWRAM 写监视（记录 PC/addr/size/value，排除 BIOS-PC 写），构建 | ☐ |
| S2 | 跑 FireRed 至同步点 0x081E40A6，产出 mgba_mem.bin + mgba_writes.bin | ☐ |
| S3 | gbars 加 Bus.write_probe 钩子（HLE/DMA 写抑制）+ examples/probe.rs，产出 gbars_mem.bin + gbars_writes.bin | ☐ |
| S4 | 对拍：先验快照差异复现（94 字节），再逐记录 diff 写日志找首个分叉 | ☐ |
| S5 | 修复根因（配单测），fmt/test/clippy 全绿 | ☐ |
| S6 | 快照 diff 迭代至 0 字节（重复 S2→S5） | ☐ |
| S7 | FireRed 300 帧：IntrMain 分发正常、DISPCNT 脱离 0x0080、出标题画面 | ☐ |
| S8 | mGBA 同帧像素对拍（mGBA dump 帧缓冲 vs 本地 BMP），PPU 细节偏差修复 | ☐ |
| S9 | 更新 README/HANDOFF（含 83→91 测试数漂移），提交 | ☐ |
| S10 | 删除本执行文档 + 删除临时探针代码 | ☐ |

## 约定

- 对拍日志格式：小端 u32 × 4 = (pc, addr, size, value)，16 字节/条。
- 双侧过滤一致：只记 IWRAM 写（addr>>24 == 0x03）、PC ≥ 0x02000000（排除 BIOS 例程/HLE 写）；DMA 写双侧不记。
- 环境坑：写源码用 Write/Edit 工具；cargo 输出重定向到文件；mGBA iwram 是 uint32_t*。

## 执行记录

（随执行追加）
