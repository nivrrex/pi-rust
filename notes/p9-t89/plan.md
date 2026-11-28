# T89（已修，F88）/ T97（未解决，F93）

## T97-footer-refresh-on-state：**已修复（F97）**

**根因（已验证）**：缓存失效与 handler 执行**顺序错了**。我在 `TuiEventSink` 的
`TurnEnd` 分支里同步发 `ExtensionUiInvalidate`，而 `turn_end` 本身走**异步串行队列**——
失效信号先到，重渲染读到的仍是旧计数。

决定性证据是那个"反常"现象：原始夹具恒 `n=0`，但**只要在 handler 里加一次
`fs.appendFileSync`（把 handler 拖慢）就变成 `n=4`**。典型的时序竞态。

**修法**：给分发队列的条目加 `invalidate_ui_after` 标志，新增
`emit_event_then_invalidate_ui`，让"派发事件"与"丢弃 UI 缓存"排在**同一条串行队列**里，
后者必然在 handler 返回之后执行。

**结果**：`K4 n=0 → n=1`，连续两次 `compare-tui same`；抽查 6 个带扩展的场景全 same。

## 此前记录（保留）

**现象**：golden 页脚最终显示 `K4 n=1`，ours 恒 `K4 n=0`。

**已确认的事实**（都有实测）：
- 夹具的 `pi.on("turn_end", () => { turns += 1 })` **确实执行了**——加日志后
  `/tmp/t97-turns.log` 记录了 `turn 1..4`，即 `turns` 真的自增。
- `turn_end` 事件确实到达 runner（`ext.event` 日志里在 `agent_end` 之前一条）。
- 因此这不是派发断裂，而是**画面没跟上**：pi-rs 按 `(width, digest)` 缓存扩展 UI 的
  渲染结果，而 digest 只涵盖**宿主可见**状态，看不到扩展的私有变量（`turns`）。
  ref 是**每帧**调组件 `render(width)`，所以它天然能看到。

**已做但没解决**：新增 `TuiEvent::ExtensionUiInvalidate`，在 `AgentEvent::TurnEnd`
里发出、主循环收到后清 `last_ui_render_key` 并置 `dirty`。加了之后 T97 仍是 `n=0`
（原因未查明——可能是清键之后 `ui_render_pending`/`render_ready` 的闸门又挡了一轮，
或渲染请求发出时扩展的 `turns` 已被读取成旧值）。

**保留该改动的理由**：它修的是真实的架构差异（回合边界丢弃缓存），且实测
`T89-ext-select`、`T85-paste12-w80` 等带扩展的场景仍 same、95 组单测全过。

**下一步建议**：在主循环清 `last_ui_render_key` 之后记录一次"是否真的发出了
`render_ui_block` 请求"，以及 `render_ui_block` 返回的 `lines` 内容——
区分"没发请求"与"发了但扩展返回旧值"。

## T89-ext-select：已定位并修复（F88）

## 根因（已验证）
`ext_tools::emit_ui_prompt`（F64/6 加的 `ui_prompt_start`/`ui_prompt_end` 派发）
用**同步** `host.emit_value`（10s 超时）。这两个事件正好在**面板打开/关闭的那一刻**
发出，而此刻扩展宿主正阻塞在 `ctx.ui.select(...)` 的 await 里等用户回答 ⇒
同步 RPC 把调用线程卡住；面板开始那次发生在 TUI 主循环里 ⇒ 主循环停摆，
面板画不出来、按键也处理不了，pty 超时 `helperExit=3`。

改法：换成 `enqueue_emit`（专用线程的异步队列）。返回值本来就没人用
（ref `runner.ts:568-572` 也是 `queueMicrotask` 派发、不等）。

**对照**：改回同步 → `helperExit=3`；改成异步 → `helperExit=0` 且 `compare-tui same`。
T80 / T74 同时恢复 same。

## 原记录（保留）

## 现象
`scenarios-tui/T89-ext-select` 跑出 `helperExit=3 measuredExit=-1`，raw 停在编辑器边框、
无退出序列 ⇒ TUI 未响应最后的 `\u0004`，pty 超时被杀。

## 已排除（都有实测）
1. **不是代码逻辑回归**：在 `2a90895`/`a377703`/`37f6988`/`e853a3a`/`77e4c54`
   上均为 `helperExit=0`，而 `eba5db9`/`0c916ea`（**该提交只改测试文件**）为 3。
   只改测试却能翻转 ⇒ 结果不跟随代码。反复在同一二进制上跑稳定为同一个值（各 2–3 次）。
2. **不是 `after_provider_response` 的同步 RPC**：把 `SharedSink` 的该钩子置空，仍 3。
3. **不是 `before_provider_headers`**：同样置空，仍 3。
4. **不是 `ctx_snapshot` 里 `leafId` 的逐次重读**：去掉该重读，仍 3。
5. **不是 `ctx.ui.setStatus` 的 `getExtensionStatuses`**（F76）：`helperExit=3` 在 F71 上已复现。
6. **不是输入框非空**导致 `CtrlD` 变成删字符：raw 末尾输入框为空。

## 本轮新增的关键事实（下次从这里继续）
在 `pi-cli` 主循环的 `match event` **之前**无条件记录 `TuiEvent::CtrlD`
（`F81_DEBUG` 写文件），跑到 T89 时**日志从未生成** ⇒ `CtrlD` **根本没到主事件循环**。
因此问题不在 `apply` 里的分支吞键（我曾改过 4475 行的 `Escape | CtrlD` 分支，
加 `selector.is_some()` guard，无效果，已回退），而在更上游：键盘事件的分发/读取层。

7. **新增（本轮）**：把场景最后的 `\u0004` 换成 `\u001b`（Esc，扩展面板的取消键，
   走 `ext_ask` 的 `TuiEvent::Escape | TuiEvent::Quit` 分支）后**仍然** `helperExit=3`。
   ⇒ 面板打开后**所有按键都失效**，不只是 `ctrl+d`。这把范围从"某个分支吞键"
   收窄到"`ext_ask` 打开后主循环不再消费键盘事件"。

8. **上一轮（编号 8）的三条结论里有一条是错的**，原因是日志方法用错：
   JS 侧我用 `require("node:fs")`（ESM 下不可用），异常被 `catch {}` 吞掉 ⇒
   日志文件恒空 ⇒ 误判为"事件从未到达 runner"。**用 `fs.appendFileSync` 重做后，
   全链路其实是通的**。另一条错在 Rust 侧统计：我用 `sort | uniq -c` 看事件名，
   排序掩盖了顺序，误以为 `agent_end` 缺失——按原始顺序看，它在 `agent_settled` 前一条。

9. **正确的链路事实（全部来自实际运行）**：
   - `ext_tools::emit_event` 顺序发出：`agent_start` → `turn_start` → `message_*` ×若干
     → `before_provider_request` → `turn_end` → **`agent_end`** → `agent_settled`。
   - runner 侧收到同样的事件序列，并在 `agent_end` 处理里真的调用了
     `ctx.ui.select("Pick a color")`。
   - 宿主收到 `host.ui.select`，且 `ui_ask=true`（通道已装）。
   - TUI 主循环 `asks.try_recv()` **拿到了** `kind=select`（日志 `GOT ask kind=select`）。
   - 但屏幕 raw 里**没有** `Pick a color` ⇒ 面板没渲染出来。
   - 读键线程**没有**因 `read` 返回 `Ok(0)`/`Err` 而退出。

   ⇒ 剩余缺口收窄到两处：①收到 `UiAsk` 后 `state.open_panel(...)` 到屏幕这一步；
     ②`\u0004` 未抵达 stdin/主循环（与①可能同源：面板状态不对导致后续键被丢弃）。

10. **本轮被否掉的两个猜测**（各做过对照，均无变化）：
   - "`after_provider_response` 同步 RPC 阻塞"：把 `SharedSink` 的钩子置空，仍 3。
   - "`before_provider_headers` 同步 RPC 阻塞"：同样置空，仍 3。
   - "`ctx_snapshot` 里 `leafId` 逐次重读"：去掉，仍 3。

## 上一轮的错误取证（保留以警示）
   - 在 `pi-cli` 主循环 `rx.recv_timeout` 收到事件处记录：整个 T89 只收到 **9 条**
     事件，最后一条是 `UsageTotals`，**没有任何 `CtrlD`/`Escape`** ⇒ 末步的
     `\u0004` 从未进入主事件循环。
   - 在 runner 的 `ext.event` 入口记录：**一条都没到**（日志文件未生成）⇒ TUI 路径下
     扩展事件根本没送到扩展宿主。
   - 在 `ext_tools::emit_event` 记录 `host_handle()` 是否存在 + 事件名：`host=true`
     （通道正常），实际发出的是 `message_update`×4、`message_start`×3、`message_end`×3、
     `turn_start`/`turn_end`/`before_provider_request`/`agent_start`/`agent_settled` 各 1，
     **独缺 `agent_end`**。
   - 在 runner 的 `ctx.ui.select` 入口记录：**从未调用** ⇒ 夹具的
     `pi.on("agent_end", …) → ctx.ui.select(...)` 从未执行 ⇒ 面板从未出现。
   ⇒ 收敛为：`AgentEvent::AgentEnd` 这条派发链断了（`run.rs:502` 无条件 emit，
     `TuiEventSink::emit` 的 `AgentEnd` 分支无条件派发，但两者都没观察到）。
     下一步从"`run_loop` 是否提前返回 Err"查起（TUI 的 `let Ok(produced) = result
     else { … return; }` 分支会直接结束提交线程）。
   - `parse_key` 探针确认 `0x04 → "ctrl+d"`、`0x03 → "ctrl+c"`、`0x0f → "ctrl+o"`，
     即**字节解析层没问题**，不必再查 `input.rs`。

## 建议下一步
- 先确认 `\u0004` 是否真的被 pty 写入（`--raw` 记录写侧字节），排除场景/helper 侧。
- 再在 `pi-cli` 读键线程（`event::read` 所在处，约 `tui.rs:8976`）与
  `pi_tui::input` 的 `map_key` 入口各加一次记录，确认 `\u0004` 在哪一层丢失。
- **首要**：在 `tui.rs` 的 `rx.recv_timeout`（约 `:2478`）收到事件处无条件记一行
  （事件 Debug），确认 `ext_ask` 打开后 `rx` 是否**还有**事件进来。
  - 有事件 ⇒ 问题在主循环内的分支；无事件 ⇒ 在读键线程/crossterm 层，
    再查 `event::read` 线程（约 `:8976`）是否在面板打开后被 join/阻塞。
- 检查 `ext_ask` 打开时是否有独立输入路径（`ctx.ui.custom` 走 `ext.custom.input`，
  `select/confirm/input` 走 `ext_ask`；两者与主键盘读取的关系未查清）。

## 不做的
不再用"二分到某提交"的方式归因——已证明该手段对本问题无效（只改测试也翻转）。
