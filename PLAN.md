
## F8：`user_bash` 事件（两种返回值）

- `{result}` 直接采用：扩展给 `output/exitCode/cancelled/truncated/fullOutputPath`，宿主不执行本地 bash；
- `{operations}` 替换执行：用扩展给的 operations 跑完，再按 `recordBashResult` 落盘；
- 扩展抛错时**不回退**本地执行（ref `interactive-mode.ts:6735-6745`）；
- 依赖 F7 的 `BashExecution` 消息类型（已完成）。

## F9：TUI 的 `!` / `!!`

- 输入分流（`:3225-3240`）与 bash 模式边框色；`BashExecutionComponent` 渲染（`:3739-3745`）；
- 取消（Esc）、流式期间的 pending 队列（`agent-session.ts:3509-3515`）；
- `/tree` 的命令摘要（`tree-selector.ts:577/808/910`）；
- TS 录制 golden：dark/light 各四类（成功、`!!` 不进上下文、失败退出码、取消），逐帧 same；
- （F7b 遗留项 ④ 已在 F7c 完成，见 `crates/pi-cli/tests/p9_f7c_resume_bash.rs`）

## F10：已关闭（真因：TUI/rpc 入口未登记会话路径；见 notes/decisions.md「F10 复盘」）

遗留：rpc 入口的 appendEntry（懒创建会话 ⇒ 启动期路径未知），见 LIMITATIONS。
**F12 更正**：todo 的持久化载体是 toolResult.details.state（不是 appendEntry），判据见 decisions「F12」。

## F10（历史记录，已关闭）：`p9_e3b_append` 偶发（退出时动作帧排空）

- 现象：workspace 全量并行时偶发整条 `custom` 条目丢失（assistant 已落盘）。
- 方向：收尾前把宿主动作帧排空（与 E4h 的 `in_flight`/`drain_pending` 同类），不加延迟、不放宽断言。
- 前置：先找到能稳定放大的复现手段（并发 8 份 + 循环 200 次量级）。

## F13：`/new` 与 `/resume` 的 pty 用例

- `/new` 的 ref 语义（`agent-session-runtime.ts:226-252`）：`session_before_switch("new")` →
  新 SessionManager → `teardownCurrent` → `session_start{reason:"new", previousSessionFile}`；
  UI 侧 `interactive-mode.ts:6657-6670` 追加 `✓ New session started`。
- 待做：实现 `/new`（writer 重建 + 清屏 + 事件顺序）、`/resume` 与 `/new` 各一条 pty 用例、
  TS 录制 dark/light 对拍。
- 另：`transcript_from_messages` 的 `ToolCall.id` 用 `replay-N`、`ToolResult` 分支为空，
  在有"折叠态有可视结果行"的 golden 之前不改（当前 T71 逐帧 same）。

## F17：`/new` 的 TS 对拍 golden + `/resume` 的 pty 用例（部分完成）

已完成：`/new` 的 dark/light golden（T97，逐帧 same，10/10×2）、`session_shutdown` 派发。
待做：`/resume` 的 pty 用例与 session_start/session_shutdown 的 reason 断言；退出矩阵（暂停）。

- `/new`：dark/light 各录一个场景（输入 `/new` 回车，快照切换后的屏幕），逐帧 same + 10 次稳定性。
- `/resume`：pty 用例断言切换后扩展 `appendEntry` 落到被 resume 的文件、`session_start` reason 为 `resume`。
- 已做：`/new` 的实现与 pty 用例（`p9_f16_new_session`）。

## F28（自述）
- S1 `92cb1a5a`：`ctx.sessionManager.getSessionId/getBranch/getLeafId/getSessionDir`、`ui.getToolsExpanded`；
  `session_start` 延后到会话登记后派发；`p9_f28_session_id`。
- S2 `ba312269`：jiti 纯函数模块 + pnpm 多 hash 布局；扩展标签去 `/./`；`tools/difftest/test/jiti.test.mjs`（8 条）。
- S3+S4 `5953dc6e`：`[Extension issues]` 折行（`p9_f28_issue_wrap` 80/120 列）；shim `ToolExecutionComponent.updateDisplay`。
- 未做：底栏 `?/0` 取证、`[Extension issues]` pty 截图、`custom_input` 用例的并行超时治理。

## F29（自述）
- S4d `6ca5f3b8`：`session_start` 之前就填好 `ctx.getContextUsage()`（tokens=system 估算、window=模型
  `contextWindow`、percent=tokens/window*100），真机底栏 `?/0 (auto)` 的根因是扩展读到 `undefined`。
  新测 `p9_f29_ctx_usage`（断言 window=200000、percent 为数字、tokens>0）。
- S5 `4ed85f8d`：`custom_input` 的 5s 超时是竞态——invoke 在队列里时按键 Cmd 排在它后面（actor 单线程），
  或走直连时 actor 恰好空闲没人读帧。修法：`active_invokes` 计数决定走直连 + 直连后补发 `Cmd::Drain`
  让 actor 读一轮。复测：并行 12 轮 0 失败（修前约 25% 失败）。

## F31-B 收口记录
- 落盘门控对齐 ref（`session-manager.ts:1160-1185`）：无 assistant 不落盘；`!`-only 会话不产生 jsonl、不打印 resume 行。已验证。
- `user_bash` 事件三种返回：`{result}` 采用、`{operations}` 未接入（登记）、handler 抛错 fail-closed。已验证。
- `!` 场景 golden 暂不入库（3 类差异未收敛，见 LIMITATIONS 第 5 条）。
- C1：`!` 后 footer 改用 `estimate_context_tokens`（ref `compaction.ts:218-246`），与 TS 逐帧一致。
- C3：入库 3 个 `!` 场景 golden（`T89a/T89b/T89c`）；`T89d` 折叠高度差 1 行未入库。
- C4：发布包 `~/archives/publish-20261106c`（buildHead `638ccc27`、publishTree `f98badae`，未发布）。
- C2：折叠态在输出与提示之间补一行空行（与 TS 逐行对拍得出），`T89d-bash-fold` 转 same 并入库；regress dark **88/0/20**、light **23/0/1**。

## F32 收口
- Markdown 对齐 ref：嵌套列表缩进/换行、H3+ 保留 `###` 前缀、有序序号、链接 URL、围栏语言、引用块 ` │ `、分隔线、GFM 表格。
- Windows：`PATH` 同时找 `node`/`node.exe`（`find_node_in_path` 纯函数 + 单测）。
- frontmatter：复杂 YAML 只跳过该键、不再整条报错。
- regress 新增 `STYLE_CHECK=1`（接入既有 compare-style，默认关闭）。

## F34 收口
- Windows：jiti 覆盖 `%LOCALAPPDATA%\pnpm\global\<ver>\<hash>` 等布局（多 hash 取最新）、失败提示列全候选；node 查找双后缀。
- Markdown：GFM 表格列宽按 ref 用自然宽、行内强调/删除线逐段染色、H1 underline、任务列表标记、列表内代码块缩进、HTML 按纯文本。
- 工具：`tools/difftest/{md-oracle,md-diff,color-dump,color-classify}.mjs` + `crates/pi-tui/examples/md-render.rs`。
- 新增入库 golden：`T89e-md-table`、`T89f-md-elements`、`T89g-md-table-cjk`。
- 发布包 `~/archives/publish-20261107e`（buildHead/publishCommit `4bb46bf2`、publishTree `54c80ef7`，未发布）。
