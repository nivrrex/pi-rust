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

## Q1 失败用例（改前 FAILED → 改后 ok）
`crates/pi-cli/tests/p9_f42_ext_logs.rs`：pty 跑真实二进制 + 合成扩展（load / `session_start` /
延迟 300ms 三处各写 `stderr` 与 `console.log`），断言「`?2004l` 之前的画面里不含 `[noisy]`」
且「6 条都进 `<agentDir>/extensions.log`」。
改前失败原文：`扩展日志污染了 TUI 画面：["[noisy] stderr-at-load", "[noisy] stdout-at-load", "[noisy] stderr-at-session-start"]`

## Q2 修复
- `HostOptions` 新增 `log_path: Option<PathBuf>`：
  - `Some(path)`（TUI 模式）= 宿主把子进程 stdout/stderr **追加**到该文件；
  - `None`（print/rpc）= 仍直接写宿主 stderr。
- 路径：`<agentDir>/extensions.log`（`ext_tools::extension_log_path`，基于配置目录，不硬编码）。
- 退出时 `ext_tools::flush_extension_log(&agent_dir)` **一次性**输出到 stderr，
  位置在 `guard.restore()` **之后**（否则会被最后一帧 diff 覆盖）。
- 判断"是否 TUI"：`has_ui_frontend()`（`UI_TX` 已注册）。
- tty 脱离保持：宿主 fd 仍是管道（`p9_f41_host_no_tty` 通过）。

## Q3 复测
- `p9_f42_ext_logs` ok；`p9_f41_host_no_tty` ok；`p9_f39_exit_timeline` ok；
  `p9_f36_exit_terminal` 3/3；`p9_f38_ctrlc_double` 2/2。
- strace（TUI 模式、带 noisy 扩展）：`TCSETS` 只剩 pi-rs 自己的 pid 两次，**无 node 的 TCSETS**。
- print 模式（`--list-tools`）：4 条 `[noisy]` 仍直接到 stderr（转发未改），日志文件也照写。
- golden：T01/T05/T16/T90/T100/T103/T92/T89i/T91 全 same。

## 影响面（登记）
- TUI 模式下扩展日志**不再**实时出现在终端，改为退出时一次性输出；文件保留（不删除）供回看。
- print/rpc 模式行为不变。
- 与 ref 的差异：ref 在 TUI 下**不**拦截扩展 stdout/stderr（Q0 已证，6/6 行都进画面），
  我们主动拦截 ⇒ 这是**有意偏离**（改善可读性），已在此登记。
