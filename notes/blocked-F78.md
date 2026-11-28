# blocked-F78：`agent_before_settle` 的续跑（TUI）

## 目标
`should_continue` 为真时用当前上下文再跑一轮（ref `agent-session.ts:1546-1549`）。

## 状态
**未实现，已回退**。HEAD `7644d35`（F77）保持"派发 + entries 落盘、不续跑"，
LIMITATIONS 已如实登记。

## 已尝试（3 次，均失败于结构改动）
1. 在 `let produced = loop {`（retry 循环）**外**再套 `'settle: loop`，
   settle 判定在其中；`continue 'settle` 时用 entries 扩充 `messages`。
   → 编译过，但**漏了退出路径**：不续跑时循环空转，settle 被反复调用
   （实测 `call:1..call:12`），且未续跑的对照用例也"通过"（判据失效）。
2. 补 `break 'settle;` 放在 `emit_settled()` 前。
   → 收尾段（emit_settled / 会话落盘 / 并入历史）随之落入循环体内，
   `break` 后的代码成为 unreachable（clippy `-D warnings` 报错），
   且 `history_size` 赋值在续跑分支里不再被读。
3. 试图把收尾段整体移出循环：`let produced;` 前置声明、循环内 `produced = produced_now;`、
   收尾段减 4 格缩进，并删掉原用于闭合 `'settle: loop` 的那行 `}`。
   → 脚本按 `emit_settled` 定位时匹配到了 **Err 分支里的那一处**，
   误把 5622-5914 整段减了缩进，括号/缩进全乱（`return;` 错位）。

## 根因判断
不是逻辑难，是**该文件这一段有 3 处同名的 `concrete_sink.emit_settled();`**
（Err 分支、Ok 分支、以及我新增的），任何按文本定位的批量缩进都会选错。
另外这段嵌在
`TuiAction::Submit` 闭包 → `if let Some(parts)` → 线程闭包 → retry loop 四层缩进里，
手写缩进极易错位。

## 建议下一步
- 不要再用文本定位做缩进搬移。改为**先抽函数**：把「settle → 落盘 → 并入历史」
  抽成 `fn finish_turn(...)`，再在循环里调用它，避免大段搬移。
- 或先写一个 `rustfmt` 能校验的小步改动（每次改一处、`cargo fmt --check` 立即验），
  不要一次性搬 300 行。
- 抽函数时注意：`produced`/`messages`/`history_size`/`concrete_sink`/`parts`
  都是闭包捕获变量，抽函数要改成显式参数或传 `&mut`。
