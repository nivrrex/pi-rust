# F44 清单

起点 HEAD `3d8a646f`（F43 收口）。用户已授权扩大范围：S0 = F43-R1 的并发修复。

- [ ] S0 日志按实例输出（并发安全）
- [ ] S1 markdown 列表：续行悬挂缩进 / 段落紧跟列表的空行
- [ ] S2 恢复会话：底栏 R/CH 与 Todo 部件
- [ ] S3 窗口标题
- [ ] S4 原生页脚 cwd 家目录缩写
- [ ] S5 收口

## S0（并发日志，已修）
失败用例 `p9_f44_log_instances` 改前 FAILED（A 退出输出里混进 `[marker] B start`）。
修法：宿主两条管道 → 单一 mpsc → 单写入任务，按**整行** `O_APPEND` 落盘并把同一行存入
本实例 `LogBuffer`（上限 256 KiB，丢最旧并在输出前提示丢弃行数）；退出只输出本实例缓冲。
F43 的连跑用例（`p9_f43_log_delta`）无回归。启动截断改「临时文件 + rename」，
并发窗口内可能丢极少量行（已登记）。

## S1（markdown 列表，已修）
根因：`wrap_markdown_line` 的列表续行只沿用行首 1 空格。ref `markdown.ts:758-781`：
首行前缀 `indent + listBullet(marker)`、续行 `indent + " ".repeat(visibleWidth(marker))`。
修法：新增 `list_marker_len`（含任务标记 `[x] `），续行补齐到内容起始列。
S1b（段落紧跟列表）：实测 TS 与 pi-rs **都不补空行**，`md-diff --ignore-blank` 110/80 列均 `different=0` ⇒ 不改。
新语料 `md-corpus/list-hang.md` + `T106-md-list-hang` golden（`same`）。

## S2（恢复会话，已修，两处实证根因）
1. **缺 `R/W/CH`**：恢复路径用 `settings_selector::session_usage`（只有 input/output/cost）。
   改 `session_writer::session_usage_totals`；并在 `TuiEvent::Usage` 分支补 `R/W/CH` 拼接
   （此前只有 `UsageTotals` 分支拼这三段，ref `footer.ts:134-137`）。
2. **缺 Todo 部件**：TUI 的 `-c`/`--session` 恢复分支**不写 system 条目** ⇒
   `append_system_message` 里那次 `flush_pending_session_start()` 永不执行 ⇒ 扩展收不到
   `session_start`（`pi-handoff-gate` 正是在该事件里 `reconstructTodo` + `setWidget`）。
   修法：恢复分支显式 `flush_pending_session_start()`（幂等）。
结果：底栏 `↑100 ↓20 R40 W10 CH26.7% $0.002 0.1%/128k (auto)` 与 `Todo ▱▱▱▱ 0/2`
均与 TS 一致；`T107-resume-usage-cache` golden `same`。

## S3（窗口标题，已修）
ref `interactive-mode.ts:1065-1073` + `terminal.ts:520-523`：`setTitle` 用 **OSC 0**，
格式 `${APP_TITLE} - ${cwdBasename}`（`APP_TITLE` 默认 `π`，`config.ts:503`）。
pi-rs 改前：启动**不发**任何标题 OSC，且 `title_osc` 用 OSC **2**。
改后：启动首帧写 `\x1b]0;π - work\x07`，OSC 改 0。单测同步更新。
Windows 侧（conhost 是否需额外开 VT）**未验证**，写入真机清单。

## S4（页脚 cwd 缩写，已修）
根因：`main.rs` 解析出 `home` 却**从未写进 `state.home`** ⇒ 页脚永远显示完整路径。
修法：`state.home = Some(home)`（`home_from` 已按 `HOME` → `USERPROFILE` 解析）。
实测：`HOME=/tmp/s4home` 与「只设 `USERPROFILE`」两种情形都输出 `~/Desktop/OA-Mails`。
Windows 路径分隔符/大小写规则登记 LIMITATIONS。
