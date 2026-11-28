# spec/h-rpc.md — rpc 协议

来源 `packages/coding-agent/src/modes/rpc/{rpc-types,rpc-mode,jsonl}.ts`
与 `packages/coding-agent/src/modes/json-event.ts`。

## 1. 传输与帧格式

- 协议总述：'Commands: JSON objects with `type` field, optional `id` for correlation'；'Responses: JSON objects with `type: "response"`；'Events: AgentSessionEvent objects streamed as they occur'。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:1-12〕
- 帧格式是**严格 JSONL，只按 LF 切分**：载荷字符串里可以包含 U+2028、U+2029 等 Unicode 分隔符，客户端必须只按 `\n` 切记录。〔packages/coding-agent/src/modes/rpc/jsonl.ts:4-12〕
- 读取器刻意不用 Node 的 `readline`，因为它会在合法 JSON 字符串内部的那些 Unicode 分隔符上切分，不满足严格 JSONL。〔packages/coding-agent/src/modes/rpc/jsonl.ts:14-21〕
- 读取器用 `StringDecoder("utf8")` 处理分块，因此跨块的多字节字符不会损坏；每行末尾的 `\r` 会被剥掉。〔packages/coding-agent/src/modes/rpc/jsonl.ts:21-30〕
- 流结束（`onEnd`）时若 `buffer.length > 0`，残留内容作为最后一行发出。〔packages/coding-agent/src/modes/rpc/jsonl.ts:43-49〕
- 写出会先 `takeOverStdout()` 接管 stdout，所有输出经 `writeRawStdout(serializeJsonLine(obj))`；每次写完等 `waitForRawStdoutBackpressure()` 处理背压。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:54-62〕〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:787-790〕
- 命令的关联方式是可选的 `id` 字段，原样回显在响应里。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:7-10〕

## 2. 命令（stdin）

- `RpcCommand` 的每个成员都带 `type`，`id` 可选。分组与成员如下。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:20-74〕
- 提示类：`prompt`（带 `message`、`images?`、`streamingBehavior?: "steer" | "followUp"`）、`steer`、`follow_up`、`abort`、`clear_queue`、`new_session`（带 `parentSession?`）。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:21-27〕
- 状态类：`get_state`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:29-30〕
- 模型类：`set_model`（`provider` + `modelId`）、`cycle_model`、`get_available_models`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:32-35〕
- thinking 类：`set_thinking_level`（`level: ThinkingLevel`）、`cycle_thinking_level`、`get_available_thinking_levels`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:37-40〕
- 队列模式类：`set_steering_mode`、`set_follow_up_mode`，`mode` 取值都是 `"all" | "one-at-a-time"`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:42-44〕
- 压缩类：`compact`（`customInstructions?`）、`set_auto_compaction`（`enabled`）。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:46-48〕
- 重试类：`set_auto_retry`（`enabled`）、`abort_retry`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:50-52〕
- bash 类：`bash`（`command`、`excludeFromContext?`）、`abort_bash`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:54-56〕
- 会话类：`get_session_stats`、`export_html`（`outputPath?`）、`switch_session`（`sessionPath`）、`fork`（`entryId`）、`clone`、`get_fork_messages`、`get_entries`（`since?`）、`get_tree`、`get_last_assistant_text`、`set_session_name`（`name`）。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:58-68〕
- 消息类：`get_messages`；命令枚举类：`get_commands`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:70-74〕
- `RpcCommandType` 是 `RpcCommand["type"]` 的提取类型，供 switch 收窄用。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:293-297〕

## 3. 响应（stdout）

- 成功响应统一形状：`{ id?, type: "response", command, success: true }`，有数据时再加 `data`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:64-73〕
- `prompt` / `steer` / `follow_up` / `abort` 的成功响应不带 `data`，注释说明它们是异步的，随后由事件流给出进展。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:116-121〕
- `clear_queue` 的 `data` 是 `{ steering: string[]; followUp: string[] }`；`new_session` 的 `data` 是 `{ cancelled: boolean }`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:122-129〕
- `get_state` 的 `data` 是 `RpcSessionState`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:131-132〕
- 错误响应是统一的一个成员，带 `command: string` 与 `error: string`，注释写明任何命令都可能失败（`any command can fail`）。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:238-239〕
- 构造错误的辅助函数就是 `{ id, type: "response", command, success: false, error: message }`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:75-76〕
- 未识别的命令走 `default` 分支，错误文本 `Unknown command: ${unknownCommand.type}`，且 `command` 字段填的是这个未知类型名。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:715-718〕
- 输入行 JSON 解析失败时，响应的 `id` 是 `undefined`、`command` 是字面量 `"parse"`，错误文本 `Failed to parse command: ...`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:752-766〕
- 命令处理抛异常时，响应带 `command.id` 与 `command.type`，错误文本取异常的 `message`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:792-801〕
- `set_model` 找不到模型时错误文本是 `Model not found: ${command.provider}/${command.modelId}`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:476-476〕
- `get_entries` 的 `since` 指向不存在的 entry 时错误文本是 `Entry not found: ${command.since}`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:644-644〕
- `set_session_name` 传空名时错误文本是 `Session name cannot be empty`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:664-664〕
- `clone` 在没有选中 entry 时错误文本是 `Cannot clone session: no current entry selected`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:624-624〕
- rpc 模式下不支持主题切换，固定返回 `Theme switching not supported in RPC mode`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:300-300〕

## 4. 事件（stdout）

- 事件就是 `AgentSessionEvent`，经 `session.subscribe(...)` 订阅，输出前统一过 `toJsonEvent(event)`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:353-356〕
- `toJsonEvent` 的投影规则（剥掉 `partial`、`toolcall_start` 补 id/toolName、非 assistant 角色报错）见 `spec/b-agent-loop.md` 第 3 节。〔packages/coding-agent/src/modes/json-event.ts:40-61〕
- `JsonAgentSessionEvent` 是把 `message_update` 换成 `JsonMessageUpdateEvent` 之后的联合，其余成员原样。〔packages/coding-agent/src/modes/json-event.ts:10-18〕
- **已验证**（P4）：`abort` 的响应**排在该轮所有事件之后**——ref 是 `await session.abort()` 拿到结果后才回响应〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:428-431〕
- **已验证**（P4）：`set_thinking_level` 走 `session.setThinkingLevel(...)` 后回响应〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:499-502〕
- **已验证**（P4）：切换等级会**先发** `thinking_level_changed` 事件〔packages/coding-agent/src/core/agent-session.ts:2269〕
- **仍未证实**：运行中 `prompt` 被拒时错误响应的**交错位置**（TS 异步发出、落在该轮初始事件之后；我们同步返回、排在 `agent_start` 之前；语义与文案已逐字一致）〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:394-408〕

## 5. 状态

- `RpcSessionState` 字段：`model?`、`thinkingLevel`、`isStreaming`、`isCompacting`、`steeringMode`、`followUpMode`、`sessionFile?`、`sessionId`、`sessionName?`、`autoCompactionEnabled`、`messageCount`、`pendingMessageCount`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:96-109〕
- `get_state` 的响应里 `messageCount` 取 `session.messages.length`（已物化的对话），`pendingMessageCount` 取 `session.pendingMessageCount`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:450-471〕
- `messages` 就是 `agent.state.messages`（已处理的消息）。〔packages/coding-agent/src/core/agent-session.ts:1303-1305〕
- `pendingMessageCount` 是 `_steeringMessages.length + _followUpMessages.length`（**排队中**）。〔packages/coding-agent/src/core/agent-session.ts:2054-2056〕

## 6. 扩展 UI 子协议

- 需要用户输入时向 stdout 发 `extension_ui_request`，必带 `id` 与 `method`。`method` 取值：`select`、`confirm`、`input`、`editor`、`notify`、`setStatus`、`setWidget`、`setTitle`、`set_editor_text`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:245-281〕
- `select` 带 `title` 与 `options: string[]`；`confirm` 带 `title` 与 `message`；`input` 带 `title` 与可选 `placeholder`；前四种都可带 `timeout?`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:247-256〕
- `notify` 带 `message` 与 `notifyType?: "info" | "warning" | "error"`；`setStatus` 带 `statusKey` 与 `statusText: string | undefined`；`setWidget` 带 `widgetKey`、`widgetLines: string[] | undefined` 与 `widgetPlacement?: "aboveEditor" | "belowEditor"`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:258-279〕
- 客户端回 stdin 的 `extension_ui_response` 三种形态：`{ value: string }`、`{ confirmed: boolean }`、`{ cancelled: true }`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:287-291〕
- 关联方式：按 `id` 在 `pendingExtensionRequests` 里查；查到才 resolve 并从表里删除，查不到直接忽略（不产生响应）。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:768-782〕
- `extension_ui_response` 在命令分派之前被拦截，因此不会被当成 `RpcCommand` 处理。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:768-784〕

## 7. 生命周期与退出

- `runRpcMode` 的返回类型是 `Promise<never>`，末尾 `return new Promise(() => {})` 保持进程存活，直到显式退出。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:54-54〕〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:819-821〕
- stdin 收到 `end` 事件触发 `shutdown()`（默认退出码 0）。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:804-807〕
- `shutdown` 用 `shuttingDown` 标志防重入（重入时直接 `process.exit`），清理 signal handlers、退订事件、释放 runtime、暂停 stdin；信号不是 `SIGTERM` 时先 `await flushRawStdout()` 再退出。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:728-745〕
- 每处理完一条命令都会检查 `checkShutdownRequested()`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:747-750〕〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:791-791〕
- `registerSignalHandlers` 在 `SIGTERM`（非 win32 还含 `SIGHUP`）时先 `killTrackedDetachedChildren()`，再 `shutdown(signal === "SIGHUP" ? 129 : 143, signal)`；处理器登记在 `signalCleanupHandlers` 以便清理。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:366-380〕

## 未覆盖 / 不确定

- **已验证**（P4）：`get_state` 的 `data`（`RpcSessionState`）已实现并对拍——`model` 是完整模型对象、`thinkingLevel` 是实时等级、`sessionFile`/`sessionId` 取自会话文件〔packages/coding-agent/src/modes/rpc/rpc-types.ts:132〕
- **仍未证实**：其余命令的 `data` 结构尚未实现（`compact`、`bash`、`get_session_stats`、`get_entries`、`get_tree`），S09–S11 落地前需补齐〔packages/coding-agent/src/modes/rpc/rpc-types.ts:179-213〕
- `session.subscribe` 那次订阅把事件写出去（`output(toJsonEvent(event))`），并在 `agent_settled` 时检查 shutdown。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:355-360〕
- `session.agent.subscribe` 的那次订阅**不产生任何事件**：它只 `await waitForRawStdoutBackpressure()`，用途是等 stdout 背压。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:361-363〕
- `RpcSlashCommand.source` 是 `"extension" | "prompt" | "skill"` 三选一，另有 `sourceInfo: SourceInfo`。〔packages/coding-agent/src/modes/rpc/rpc-types.ts:81-90〕
- `get_commands` 的填充顺序：扩展注册命令（`source: "extension"`，带 `command.sourceInfo`）→ 提示模板（`source: "prompt"`，带 `template.sourceInfo`）→ 资源加载器的技能（名字加前缀 `skill:`）。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:683-712〕
- `switch_session` 会 `runtimeHost.switchSession(command.sessionPath)`，**未被取消时** `rebindSession()`，响应 `success(id,"switch_session",result)`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:605-611〕
- `fork` 会 `runtimeHost.fork(command.entryId)`，未被取消时 `rebindSession()`，响应 `{text: result.selectedText, cancelled: result.cancelled}`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:613-619〕
- `clone` 取 `session.sessionManager.getLeafId()`；无 leaf 时 `error(id,"clone","Cannot clone session: no current entry selected")`，否则 `runtimeHost.fork(leafId, {position:"at"})`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:621-631〕
- `get_fork_messages` 返回 `getUserMessagesForForking()` 的结果 `{messages}`；`get_tree` 返回 `{tree, leafId}`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:633-635〕
- `get_entries` 返回 `{entries, leafId}`：条目**不含** `type=="session"` 的文件头；带 `since` 时只取该 id **之后**的部分，找不到报 `Entry not found: <since>`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:638-648〕
- `get_messages` 直接回 `{messages: session.messages}`（**已物化**的消息列表）。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:674-676〕
- `new_session` 取可选的 `command.parentSession` 组成 options，调 `runtimeHost.newSession`；未取消则 `rebindSession()`，响应是该调用的结果（`{cancelled}`）。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:437-444〕
- `switch_session` 前先 `SessionManager.open(sessionPath, undefined, options?.cwdOverride)`，再用 `assertSessionCwdExists(sessionManager, this.cwd)` 校验，最后 teardown 并重建 runtime；返回值只有 `cancelled`。〔packages/coding-agent/src/core/agent-session-runtime.ts:203-217〕
- `get_last_assistant_text` 回 `{text}`；`text` 取自 `getLastAssistantText()`：在消息列表里**从后往前**找第一条 assistant，跳过 `stopReason === "aborted"` 且内容为空的，找不到则 `undefined`（JSON 里该键会消失）。〔packages/coding-agent/src/core/agent-session.ts:3972-3984〕
- `fork` 的默认 `position` 是 `"before"`；选中条目不存在时报 `Invalid entry ID for forking`。〔packages/coding-agent/src/core/agent-session-runtime.ts:266-275〕
- `position === "at"` 时 `targetLeafId = selectedEntry.id`；否则要求条目是 `type === "message"` 且 `message.role === "user"`（不满足时**同样**报 `Invalid entry ID for forking`），并取 `targetLeafId = selectedEntry.parentId`、`selectedText = extractUserMessageText(content)`。〔packages/coding-agent/src/core/agent-session-runtime.ts:279-287〕
- 持久化会话的 fork 走 `SessionManager.open(currentSessionFile, sessionDir)` → `createBranchedSession(targetLeafId)`（`targetLeafId` 为空时改走 `newSession({parentSession: currentSessionFile})`）。〔packages/coding-agent/src/core/agent-session-runtime.ts:304-334〕
- `createBranchedSession(leafId)` 取 `getBranch(leafId)`（root→leaf 的路径），去掉 `label` 条目并**重连** `parentId`，再写新会话文件。〔packages/coding-agent/src/core/session-manager.ts:1625-1665〕
- 另一个入口是静态方法 `forkFrom`（CLI 侧的调用点见 main.ts:349）。〔packages/coding-agent/src/core/session-manager.ts:1812-1830〕
- 该入口写的新 header 里含 `parentSession`，并用 `writeFileSync` 落盘，随后复制源会话的全部非 header 条目。〔packages/coding-agent/src/core/session-manager.ts:1840-1860〕
- `export_html` 调 `session.exportToHtml(command.outputPath)`，响应是 `{path}`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:600-603〕
- `exportSessionToHtml` 的默认输出路径：未给 `outputPath` 时用 `APP_NAME`-session-`sessionBasename`.html（相对路径，即在 cwd 下），随后 `writeFileSync` 并返回该路径。〔packages/coding-agent/src/core/export-html/index.ts:274-281〕
- 产物是**自包含** HTML：模板 `template.html`/`template.css`/`template.js` 与 vendor 的 `marked.min.js`/`highlight.min.js` 都在生成时读入内联，**不依赖网络**。〔packages/coding-agent/src/core/export-html/index.ts:145-149〕
- `shutdownRequested` 由运行时回调 `shutdownHandler` 置位（**不是**某条 rpc 命令）。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:345-347〕
- `checkShutdownRequested()` 在未置位时直接返回，否则 `await shutdown()`；它在 `agent_settled` 之后被调用。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:747-750〕
- 事件流与命令响应共用同一条 stdout 通道：`session.subscribe` 的回调里直接 `output(toJsonEvent(event))`。〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:355-360〕
- 实测（difftest 驱动 rpc 场景）：`prompt` 的响应之后**紧接着**就是 `agent_start` 与事件流，中间没有其它输出。〔golden/S28/events.jsonl:5-6〕
