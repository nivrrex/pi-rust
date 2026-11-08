# F43 清单

起点 HEAD `2a14aaa7`（F42 收口）。

## R0 失败用例（改前 FAILED → 改后 ok）
`crates/pi-cli/tests/p9_f43_log_delta.rs` + `tools/difftest/tui-two-sessions.py`：
在同一 agent 目录下连跑两次会话（每次启动扩展打一条唯一标记行），断言
「第一次退出输出 1 行」「第二次退出**只**输出本次新增的 1 行且不同于第一次」
「`extensions.log` 仍累积保留 2 行」。
改前失败原文：`第二次退出只应输出**本次**新增的 1 行，实际 2 行：["[noisy] boot 21550-…", "[noisy] boot 21568-…"]`

## R1 修复与取舍
- 新增 `prepare_extension_log(agent_dir)`：在**宿主启动前**记录文件当前长度作为本次起点偏移。
- `flush_extension_log` 改为 `File::open` + `seek(offset)` + 只读剩余部分。
- **取舍：用"偏移"而不是"实例内缓冲"**。理由：
  ①宿主日志由宿主侧两个异步读取任务（stdout / stderr 两条管道）写文件，实例内缓冲需要在
    两条并发流之间加锁与排序，才能保证输出顺序与文件一致；偏移法不引入共享状态。
  ②并发多实例同写一个文件时，偏移法天然满足"每个实例只输出自己那一段"（各自 seek 自己的起点），
     缓冲法反而要靠实例标识把行分类，行序还会交错。
  ③文件仍是唯一事实来源（进程崩了也不丢已写内容）。
  代价：若两个实例的写入区间在文件里交错，各自输出的内容会包含对方穿插进来的行 ——
  这是并发追加的固有限制，比"整文件重打"轻得多，登记为已知限制。
- 容量治理：文件超过 **1 MiB** 时，在启动时截断为**最近 256 KiB**（从下一个换行开始，避免切半行），
  随后重设偏移。登记为有意行为（否则日志无限增长）。

## R2 对拍（TS vs pi-rs，同一 agent 目录）
| 项 | TS | pi-rs 改前 | 结论 |
| --- | --- | --- | --- |
| 底栏 cwd 后的分支名 | `/tmp/r2repo (recover)` | `/tmp/r2repo (recover)` | 无差异（ref `footer.ts:118-121` + `footer-data-provider.ts:239-251`） |
| 无助手消息时 `ctrl+x` 的提示 | ` Error: No agent messages to copy yet.` | ` No agent messages to copy yet.`（dim 状态行） | **有差异，已修** |
- 差异根因：ref `interactive-mode.ts:6392` 走 `showError`（`:4463-4467`：`Spacer(1)` + `Text(theme.fg("error", "Error: " + msg), outputPad, 0)`），
  我们走了 `Status`（dim、无前缀）。改法：改派 `ExtensionNotice{kind:"error"}`（已有路径按 ref 加前缀 + error 色）。
- 新 golden `T104-copy-no-messages`（TS 录制，`same`）。`-ne` 启动屏无该行（两侧都只在按键后才出现）。

## R3 收口
见最终回复。
