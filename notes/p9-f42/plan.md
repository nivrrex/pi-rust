# F42 清单

起点 HEAD `e4af0181`（F41 收口）。

## Q0 取证（已完成）
夹具：`/tmp/Q0.json` —— 合成扩展 `noisy` 在 load / `session_start` / 延迟 300ms 三处各写
一条 `process.stderr.write` 与一条 `console.log`，共 6 行。

| 实现 | raw 中 `[noisy]` 出现数 | 画面（100×30，idle 快照） |
| --- | --- | --- |
| TS（`PI_TS_SHORT`） | **6**（stderr 3 + stdout 3） | 前 4 行直接压在屏幕顶端，`[Extensions]` 段与分隔线之间被穿插 |
| pi-rs F41（管道 + 转发到自身 stderr） | **6** | 同上（前 4 行在 `pi v0.87.1` 之上） |

⇒ **ref 在 TUI 模式下并不拦截扩展的 stdout/stderr**：`interactive-mode.ts` 全文没有对
`process.stdout/stderr` 或 `console` 的重定向/劫持（grep `redirectConsole|patchConsole|process.stdout =`
无命中），TUI 只是自己往 stdout 写帧，扩展的输出便与帧交织。
pi-rs F41 的表现与 TS **一致**（都污染画面），差别仅在行数/位置细节（TS 4 行、我们 4 行）。

结论：Q2 的"按 ref"退化为"ref 无明确行为"，因此按批次给的默认去向实现：
**TUI 模式下写日志文件 + 退出时一次性输出到 stderr**；print/rpc 保持直接转发。
这比 ref 更好（不污染画面），登记为**有意偏离**。

- [ ] Q1 失败用例
- [ ] Q2 修复
- [ ] Q3 复测
- [ ] Q4 收口
