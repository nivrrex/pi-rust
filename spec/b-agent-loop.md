# spec/b-agent-loop.md — agent loop 与事件序

核心循环在 `packages/agent/src/agent-loop.ts`（低层，无状态）与
`packages/agent/src/agent.ts`（有状态包装，持队列与 abort）+ `packages/coding-agent/src/core/agent-session.ts`（产品层，接 compaction/重试/扩展边界）。

## 1. 双层循环结构

- `runLoop` 是外层 `while (true)` + 内层 `while (hasMoreToolCalls || pendingMessages.length > 0)` 的嵌套结构；外层负责 follow-up，内层负责工具调用与 steering。〔packages/agent/src/agent-loop.ts:178-182〕
- 内层每轮开头先调 `config.prepareNextTurn?.(lastCompletedTurn)`，若返回快照则整体替换 `currentContext`、`preparedMessages`、`config.model` 与 `config.reasoning`（`thinkingLevel === "off"` 映射为 `undefined`）。〔packages/agent/src/agent-loop.ts:184-199〕
- `prepareNextTurn` 之后立刻 emit `turn_start`；注意顺序是"先 prepareNextTurn，再 turn_start"。〔packages/agent/src/agent-loop.ts:200-207〕
- 只有在 `pendingMessages.length === 0` 时才再次轮询 `getSteeringMessages()`；注释解释：prepare 阶段（如 compaction）可能耗时，期间排队的 steering 要被捡起，但如果上一次轮询已经拿到消息就不再轮询，否则 one-at-a-time 模式会在同一轮投递两条消息。〔packages/agent/src/agent-loop.ts:200-205〕
- 每条待处理消息（prepared + pending 合并后经 `declareToolChanges` 处理）都先 emit `message_start` 再 emit `message_end`，然后才推入 `currentContext.messages` 与 `newMessages`；处理完立刻把 `pendingMessages` 清空。〔packages/agent/src/agent-loop.ts:209-216〕
- 每次 provider 请求前调 `prepareRequest` 钩子，入参含 `context`、`model`、`thinkingLevel` 与 `signal`，返回的 context/model/thinkingLevel 替换运行时值；该钩子不轮询队列。〔packages/agent/src/agent-loop.ts:218-238〕

## 2. 事件序

- `AgentEvent` 共 11 个成员：`agent_start`、`agent_end`、`turn_start`、`turn_end`、`message_start`、`message_update`、`message_end`、`tool_execution_start`、`tool_execution_update`、`tool_execution_end`，以及各自字段（`turn_end` 带 `message` 与 `toolResults`，`tool_execution_end` 带 `toolCallId`/`toolName`/`result`/`isError`）。〔packages/agent/src/types.ts:485-500〕
- 注释里的定义：一个 `turn` 是一次 assistant 响应加上其后的任何工具调用与结果。〔packages/agent/src/types.ts:489-491〕
- `message_update` **只对流式中的 assistant 消息**发出。〔packages/agent/src/types.ts:494-495〕
- `agent_end` 是一次 run 发出的最后一个事件，但它的 `Agent.subscribe()` 监听器仍在 run settlement 范围内，agent 只在这些监听器结束后才变 idle。〔packages/agent/src/types.ts:478-484〕
- 流式过程中的映射：`start` 事件时把 partial 推入 `context.messages` 并 emit `message_start`（带浅拷贝 `{ ...partialMessage }`）；每个增量事件用 `event.partial` 覆盖 `context.messages` 的最后一条并发 `message_update`。〔packages/agent/src/agent-loop.ts:411-438〕
- `done`/`error` 时用 `response.result()` 的最终消息替换最后一条（或在从未 `start` 时追加并补发一次 `message_start`），然后 emit `message_end` 并返回。〔packages/agent/src/agent-loop.ts:440-453〕
- 若流在没有 `done`/`error` 的情况下结束，循环外还有一条兜底路径做同样的替换/追加并 emit `message_end`。〔packages/agent/src/agent-loop.ts:457-465〕
- `message_start`/`message_update` 发出的 `message` 都是**浅拷贝**，避免 UI 侧持有被后续原地改写的对象。〔packages/agent/src/agent-loop.ts:417-417〕
- EventBus 的 `on` 会把 handler 包成 async 并 try/catch，handler 抛错只打日志 `Event handler error (${channel}):`，不影响循环；返回值是取消订阅函数。〔packages/coding-agent/src/core/event-bus.ts:18-28〕

## 3. JSON / RPC 线格式的事件投影

- `toJsonEvent` 把 `message_update` 重写为 `{ type, usage, assistantMessageEvent }`，其中 `usage` 取自 `event.message.usage`，即累计用量。〔packages/coding-agent/src/modes/json-event.ts:56-60〕
- 投影时剥掉 `partial` 字段（`WithoutPartial<T>`），把累积快照从线上协议里去掉。〔packages/coding-agent/src/modes/json-event.ts:4-8〕
- 注释说明：`message_start` 给出初始消息，delta 增量构建，`message_end` 给出最终权威消息；累积的 usage、tool-call id 与 tool name 因为体积恒定所以保留。〔packages/coding-agent/src/modes/json-event.ts:40-45〕
- `toolcall_start` 特殊处理：从 `event.partial.content[event.contentIndex]` 取出 tool call，若不是 `toolCall` 类型直接抛错 `toolcall_start content at index ... is not a tool call`；否则附上 `id` 与 `toolName`。〔packages/coding-agent/src/modes/json-event.ts:23-30〕
- 非 assistant 角色的 `message_update` 也会抛错 `message_update message is not an assistant message`。〔packages/coding-agent/src/modes/json-event.ts:52-54〕

## 4. steering 与 follow-up

- `PendingMessageQueue` 有两种 drain 模式：`all` 返回全部快照，`one-at-a-time` 只返回队首一条；`drain()` 按 `peek()` 的长度切片消费。〔packages/agent/src/agent.ts:142-172〕
- `steeringQueue` 与 `followUpQueue` 默认模式都是 `"one-at-a-time"`，可分别通过 `steeringMode` / `followUpMode` 改写。〔packages/agent/src/agent.ts:244-245〕
- `steer(message)` 进 steering 队列，`followUp(message)` 进 follow-up 队列；`hasQueuedMessages()` 是两个队列的或。〔packages/agent/src/agent.ts:296-302〕〔packages/agent/src/agent.ts:322-324〕
- steering 在**当前 turn 的工具执行结束后**被拉取：内层循环在 emit `turn_end` 之后立即执行 `pendingMessages = (await config.getSteeringMessages?.()) || []`。〔packages/agent/src/agent-loop.ts:286-294〕
- follow-up 只在"agent 本来要停"的地方拉取，即内层循环退出后；若拿到消息就把 `explicitContinuation` 清掉、放进 `pendingMessages` 并 `continue` 外层循环。〔packages/agent/src/agent-loop.ts:300-307〕
- 注释明确：当前 assistant 消息里的 `Tool calls` 不会因为 steering 消息而被跳过。〔packages/agent/src/types.ts:278-289〕
- `getSteeringMessages` 的契约是"不得抛出或 reject"，不可用时返回 `[]`。〔packages/agent/src/types.ts:285-289〕
- `getFollowUpMessages` 的契约同样是不抛出，无可用消息时返回 `[]`。〔packages/agent/src/types.ts:298-302〕
- `transformContext` 的契约是不得抛出或 reject，应返回原数组或另一个安全回退值，否则会打断低层循环而不产生正常事件序列。〔packages/agent/src/types.ts:227-240〕

## 5. finishTurn

- `finishTurn` 在 assistant 消息与全部工具结果消息都发出之后、`turn_end` 之前被调用。〔packages/agent/src/types.ts:252-253〕
- 返回 `{ action: "end" }` 直接结束 run，不再轮询队列也不再准备下一次请求；返回 `{ action: "continue" }` 保证再来一次 provider 请求；返回 `undefined` 保持常规调度。〔packages/agent/src/types.ts:254-258〕
- `continue` 的语义是"保证一次请求"，若工具结果、steering 或 follow-up 已经安排了请求则不额外增加；否则以当前上下文继续一轮。error 与 aborted 响应仍是硬退出。〔packages/agent/src/types.ts:255-258〕
- 循环侧：`decision?.action === "end"` 时 emit `turn_end` 后 emit `agent_end` 并返回；否则把 `explicitContinuation` 置为 `decision?.action === "continue"`，再拉 steering；若 `hasMoreToolCalls` 或有 steering 消息，则把 `explicitContinuation` 重置为 false。〔packages/agent/src/agent-loop.ts:285-297〕
- `explicitContinuation` 兑现的方式是"一个只有上下文、没有新消息的 turn"：外层循环在无 follow-up 且 `explicitContinuation` 为真时再 `continue` 一次。〔packages/agent/src/agent-loop.ts:309-313〕
- hard exit：assistant 消息的 `stopReason` 是 `"error"` 或 `"aborted"` 时，直接 `finishTurn` → `turn_end`（`toolResults: []`）→ `agent_end` 并 return，不执行任何工具。〔packages/agent/src/agent-loop.ts:244-255〕

## 6. agent_before_settle

- `_runBeforeSettleBoundary` 在没有任何 `agent_before_settle` 处理器时退化为 `this.agent.hasQueuedMessages()`，即扩展点不存在时行为与旧逻辑一致。〔packages/coding-agent/src/core/agent-session.ts:1531-1532〕
- 有处理器时：置 `_isBeforeSettle = true`、清 `_abortDuringBeforeSettle`，`emitBoundary({ type: "agent_before_settle", outcome: this._lastActivityOutcome }, ...)`，随后 `_commitBoundaryDrafts(result.entries)`、`_flushPendingCustomMessages()`，再以空 entries 重建一次 `finalContext`。〔packages/coding-agent/src/core/agent-session.ts:1533-1542〕
- 继续条件是 `result.continue || this.agent.hasQueuedMessages()`；若为真但 `finalContext.canContinue` 为假，则报"非法继续"并返回 false（`agent_before_settle` 分支会调 `_reportInvalidBoundaryContinuation`）。〔packages/coding-agent/src/core/agent-session.ts:1543-1549〕
- 边界事件类型定义为 `boundary: "turn_end" | "agent_before_settle"`，非法继续的告警按这两个名字区分。〔packages/coding-agent/src/core/agent-session.ts:793-793〕
- 注释说明：低层循环在 `agent_end` 之前会排空两个队列，所以由 `agent_end` 处理器排入的消息需要一次新的 run 才能触发 pre-settlement 处理器。〔packages/coding-agent/src/core/agent-session.ts:1526-1528〕

## 7. abort 语义

- `AgentSession.abort()` 的顺序固定：若 agent run 活跃则置 `_agentRunAbortRequested`、然后 `abortRetry()`、`abortCompaction()`、`abortBranchSummary()`，若处于 before-settle 阶段置 `_abortDuringBeforeSettle`，最后 `this.agent.abort()` 并 `await this.waitForIdle()`。〔packages/coding-agent/src/core/agent-session.ts:2075-2085〕
- `waitForIdle()` 在已 idle 时直接返回，否则等 `_getIdleWaitPromise()`。〔packages/coding-agent/src/core/agent-session.ts:2087-2092〕
- 并行工具执行中，已 abort 时尚未开始的调用不会执行，而是产出错误结果 `Operation aborted`（`isError: true`），并照常 emit `tool_execution_end`。〔packages/agent/src/agent-loop.ts:616-625〕
- 串行/并行两条路径在每次循环后都检查 `signal?.aborted` 并 `break`，即 abort 会截断本批剩余调用。〔packages/agent/src/agent-loop.ts:610-612〕〔packages/agent/src/agent-loop.ts:638-640〕
- `raceWithAbortSignal` 在 signal 已 abort 时立即 reject 并对原 promise 挂一个空的 catch（避免 unhandled rejection），即"停止等待但通过 settlement 继续观察被抛弃的操作"。〔packages/coding-agent/src/utils/abort.ts:13-19〕
- `abortReason` 优先返回 `signal.reason`；没有 reason 时构造一个 `name = "AbortError"` 的 `Error("The operation was aborted")`。〔packages/coding-agent/src/utils/abort.ts:1-6〕
- `operationSignal` 把可选 signal 规范成一个永不被 abort 的 signal（不施加 deadline）。〔packages/coding-agent/src/utils/abort.ts:8-11〕

## 8. 并行工具的执行与结果顺序

- 串行还是并行由两条规则决定：`config.toolExecution === "sequential"`，或本批中任一工具的 `executionMode === "sequential"`；任一成立则整批走串行。〔packages/agent/src/agent-loop.ts:512-519〕
- 默认是 `"parallel"`，其语义注释写明：预检（preflight）按顺序做，然后并发执行允许的工具，`tool_execution_end` 按工具**完成顺序**发出，工具结果消息则稍后按 **assistant 源码顺序**发出。〔packages/agent/src/types.ts:304-313〕
- 并行路径先顺序 emit 每个 `tool_execution_start` 并做 `prepareToolCall`；只有 `kind === "immediate"`（预检即失败）的调用当场 finalize，其余把执行体包成 thunk 存入数组。〔packages/agent/src/agent-loop.ts:593-614〕
- 所有 thunk 之后用 `Promise.all` 一起触发，因此并发度等于本批调用数（源码里没有并发上限常量）。〔packages/agent/src/agent-loop.ts:643-645〕
- **已验证**：`tool_execution_end` 按**完成顺序**发出，工具结果消息在 `Promise.all` 之后按 **assistant 源码顺序**发出——两者顺序不同，`types.ts:304-313` 的注释是对的。依据：并行路径把执行体包成 thunk，`await emitToolExecutionEnd(finalized, emit)` 写在 **thunk 内部**（谁先跑完谁先发）；`orderedFinalizedCalls = await Promise.all(...)` 保序，随后的 `for (const finalized of orderedFinalizedCalls) await emitToolResultMessage(...)` 才按源码顺序发结果消息。**推翻**此前"Promise.all 保序 ⇒ 发出顺序等于源码顺序"的推断——它把"事件"与"结果消息"混为一谈。〔packages/agent/src/agent-loop.ts:616-651〕
- 工具结果消息在所有执行结束后按 `orderedFinalizedCalls` 顺序逐个 `createToolResultMessage` 并 emit。〔packages/agent/src/agent-loop.ts:646-651〕
- 批次提前终止规则：`shouldTerminateToolBatch` 要求批次非空**且**每个已 finalize 的结果的 `result.terminate === true`。〔packages/agent/src/agent-loop.ts:685-687〕
- 若 assistant 的 `stopReason === "length"`，该消息里的所有工具调用都不执行，统一报错，错误文本为 `Tool call "<name>" was not executed: the response hit the output token limit, so its arguments may be truncated. Re-issue the tool call with complete arguments.`；返回的 `terminate` 为 false。〔packages/agent/src/agent-loop.ts:475-499〕
- 理由注释：流式工具参数是用尽力而为的 `JSON salvage` 解析器 finalize 的，截断的消息可能产出"能解析能校验但静默不完整"的参数，因此没有一个是安全的。〔packages/agent/src/agent-loop.ts:468-474〕
- 已验证：`finish_reason: "length"` 且工具参数**可解析但语义不完整**（`{}`，缺必填 `path`）时确实走到这条分支——产出的工具结果以 `Tool call "read" was not executed:` 开头且 `isError` 为真。〔golden/S13b-toolcall/session.jsonl:7-7〕
- 对照：`finish_reason: "length"` 但参数被截断成非法 JSON（`{"path":`）时，走不到这条分支——错误更早发生在参数解析阶段。〔golden/S13b-text/session.jsonl:1-6〕
- 工具声明与可执行集合的差异在每次请求前变成 system message 上的 `toolsAdded`/`toolsRemoved`；若待处理消息里已有 system message，其工具字段被视为意图并被替换为 committed transcript 与可执行集合之间的差值，否则在第一条非 system 待处理消息之前插入一条新的 system message。〔packages/agent/src/agent-loop.ts:322-331〕

## 9. 重试发生在哪一层

- 低层 agent loop 本身**不做**重试；`streamAssistantResponse` 直接调 `streamFunction`，失败以 `AssistantMessage.stopReason === "error"` 的形式进入事件流。〔packages/agent/src/agent-loop.ts:240-244〕
- pi-ai 提供的是单次调用级别的重试原语 `retryAssistantCall`，签名是 `(produce, policy, signal, callbacks?)`，它包住一次"产出 assistant 消息"的调用。〔packages/ai/src/utils/retry.ts:176-182〕
- `RetryPolicy` 四字段：`enabled`、`maxRetries`、`baseDelayMs`、`maxAgentDelayMs`；`maxRetries` 为 0 表示不重试，首次调用不算重试；退避为 `baseDelayMs * 2^(attempt-1)` 再叠加 jitter，单次延迟上限默认 `DEFAULT_MAX_AGENT_RETRY_DELAY_MS = 60_000`。〔packages/ai/src/utils/retry.ts:101-111〕
- 终止规则：aborted 是终止态且永不重试；`stopReason !== "error"` 直接返回；预算耗尽或 `isRetryableAssistantError` 为假时返回最后的错误消息。〔packages/ai/src/utils/retry.ts:189-205〕
- 退避睡眠期间被 abort，会把同一次响应改写成 `stopReason: "aborted"` 返回（剥掉 `errorMessage`），这样调用方不必关心取消发生在哪一步。〔packages/ai/src/utils/retry.ts:212-223〕
- `isRetryableAssistantError` 只做分类不做策略：要求 `stopReason === "error"` 且有 `errorMessage`，先排除 `NON_RETRYABLE_PROVIDER_LIMIT_ERROR_PATTERN`，再要求 `RETRYABLE_PROVIDER_ERROR_PATTERN` 命中。〔packages/ai/src/utils/retry.ts:237-242〕
- 注释要求调用方**先单独处理 context overflow**（原文跨行写作 `handle context` / `overflow separately`），再套自己的重试预算与退避。〔packages/ai/src/utils/retry.ts:228-236〕
- coding-agent 的产品层重试是另一套：`AgentSession._prepareRetry` 读 `settingsManager.getRetrySettings()`，未启用直接返回 false；`_retryAttempt` 自增，超过 `maxRetries` 时先回退计数以保留已完成次数供事后发最终失败事件，然后返回 false。〔packages/coding-agent/src/core/agent-session.ts:3379-3391〕
- 每次准备重试会 emit `auto_retry_start { attempt, maxAttempts, delayMs, errorMessage }`，其中 `errorMessage` 为空时填 `"Unknown error"`；随后 `_omitRecoveryAttempt(message)` 把失败尝试从模型投影里省掉但保留在原始历史里，再进入可 abort 的退避睡眠。〔packages/coding-agent/src/core/agent-session.ts:3393-3409〕
- 重试成功的收尾事件是 `auto_retry_end { success, attempt, finalError? }`；当 `_retryAttempt > 0` 且本轮仍以 error 结束时发 `success: false` 并把计数清零。〔packages/coding-agent/src/core/agent-session.ts:1512-1520〕
- `AgentSessionEvent` 里 `auto_retry_start` / `auto_retry_end` 是两个独立成员。〔packages/coding-agent/src/core/agent-session.ts:189-190〕
- `RetryPolicy` 注释声明它对应 coding-agent 的 `settings.retry`（`enabled`、`maxRetries`、`baseDelayMs`、`maxAgentDelayMs`），即设置项与策略是同一套字段。〔packages/ai/src/utils/retry.ts:94-99〕

## 10. 请求前的规范化顺序

- `streamAssistantResponse` 里的顺序是：`config.transformContext`（AgentMessage 级）→ `config.convertToLlm` → `normalizeContext({ messages: llmMessages })` → 解析 apiKey → 调 `streamFunction`。〔packages/agent/src/agent-loop.ts:387-406〕
- apiKey 解析优先用 `config.getApiKey?.(provider)`，取不到才回退到静态 `config.apiKey`；注释理由是 token 可能过期（短生命周期 OAuth）。〔packages/agent/src/agent-loop.ts:398-400〕

## 未覆盖 / 不确定

- 串行路径 `executeToolCallsSequential` 逐条 `for (const toolCall of toolCalls)`，每条先 `await emit(...)` 再 `await prepareToolCall(...)`——**与并发路径共用同一套准备逻辑**。〔packages/agent/src/agent-loop.ts:527-550〕
- 它的签名收 `signal: AbortSignal | undefined`，并把该 signal 依次传给 `prepareToolCall` 与 `executePreparedToolCall`；`preparation.kind === "immediate"` 表示无需执行。〔packages/agent/src/agent-loop.ts:548-568〕
- 已验证：两个模式都用 `buildProviderErrorPattern` 把字符串数组以 `|` 拼成大小写不敏感的正则。〔packages/ai/src/utils/retry.ts:3-4〕
  - `NON_RETRYABLE_PROVIDER_LIMIT_ERROR_PATTERN`（**先判**）：`GoUsageLimitError`、`FreeUsageLimitError`、`Monthly usage limit reached`、`available balance`、`insufficient_quota`、`out of budget`、`quota exceeded`、`billing`。命中即不可重试——订阅/额度上限，不是瞬时节流。〔packages/ai/src/utils/retry.ts:7-24〕
  - `RETRYABLE_PROVIDER_ERROR_PATTERN`：瞬时故障与状态码 `overloaded`、`currently experiencing high demand`、`rate.?limit`、`too many requests`、`429`、`500`、`502`、`503`、`504`、`520`、`524`、`service.?unavailable`、`server.?error`、`internal.?error`；上游包装 `provider.?returned.?error`、`exceeded request buffer limit while retrying upstream`；网络与传输层 `network.?error`、`connection.?error`、`connection.?refused`、`connection.?lost`、`other side closed`、`fetch failed`、`getaddrinfo`、`ENOTFOUND`、`EAI_AGAIN`、`upstream.?connect`、`reset before headers`、`socket hang up`、`socket connection was closed`、`timed? out`、`timeout`、`terminated`；WebSocket `websocket.?closed`、`websocket.?error`；流提前结束 `ended without`、`stream ended before message_stop`、`stream ended before a terminal response event`、`http2 request did not get a response`；服务端要求退避 `retry delay`；流中显式重试指引 `you can retry your request`、`try your request again`、`please retry your request`；末尾还有 gRPC 系 provider 的文案。〔packages/ai/src/utils/retry.ts:26-88〕
  - 判定入口 `isRetryableAssistantError`：`stopReason !== "error"` 或 `errorMessage` 为空直接 false；先测不可重试模式，再测可重试模式。该函数**只分类不重试**——调用方须先单独处理 context overflow，再用自己的预算、退避与上报。〔packages/ai/src/utils/retry.ts:236-241〕
- 已验证：`retryProviderRequest` 不实现分类，只实现退避：`maxRetries` 默认 **0**，即默认不重试；每次重试都是一次全新的 SDK 请求，因此 `X-Stainless-Retry-Count` 始终为 0。捕获后依次判断：signal 已 abort 则抛 `AbortError`；`retriesRemaining <= 0`、不是 `ProviderError`、或 `isRetryableProviderError` 为 false，则原样抛出。〔packages/ai/src/utils/provider-retry.ts:105-125〕
  - 退避 `getRetryDelayMs`：先读 `retry-after-ms` 响应头，`parseFloat` 成功即用；再读 `retry-after`，可解析为数字时按**秒**乘 1000，否则按 HTTP-date 计算 `Date.parse(...) - Date.now()`；两者都没有时用 `min(0.5 * 2^retryIndex, 8) * 1000`，再乘 `1 - Math.random() * 0.25` 的抖动。〔packages/ai/src/utils/provider-retry.ts:51-65〕
  - 服务端要求的退避超过上限时不是截断而是**抛错**：`validateServerRetryDelayMs` 在 `delayMs > maxDelayMs` 时抛 `Server requested Ns retry delay (max: Ms). …`；上限默认 `DEFAULT_MAX_RETRY_DELAY_MS = 60_000`。该文案含 `retry delay`，会被上面的 `RETRYABLE_PROVIDER_ERROR_PATTERN` 命中，交回外层重试策略。〔packages/ai/src/utils/provider-retry.ts:1-49〕
  - `abortableSleep` 用 `AbortSignal` 中断退避——这是它存在的唯一理由：OpenAI/Anthropic SDK 自带的重试计时器不理会请求的 `AbortSignal`，所以调用方必须以 `maxRetries: 0` 调 SDK。〔packages/ai/src/utils/provider-retry.ts:73-104〕
- **已验证**（P4）：`AgentSessionEvent` 的这条成员列表已抽全，且其中的 `queue_update`、`entry_appended`、`thinking_level_changed` 已实现并对拍〔packages/coding-agent/src/core/agent-session.ts:164-181〕
- **仍未证实**：同一列表里 `session_info_changed` 与 `compaction_start`/`compaction_end` 的触发时机（属 P5/P6）〔packages/coding-agent/src/core/agent-session.ts:164-181〕
- 已验证（第 8 节那条"顺序矛盾"已实测定性）：`tool_execution_end` 按**完成顺序**发出，工具结果消息按 **assistant 源码顺序**发出；依据是 `emitToolExecutionEnd` 写在并发 thunk **内部**，而结果消息在 `Promise.all` 之后按 `orderedFinalizedCalls` 顺序发。〔packages/agent/src/agent-loop.ts:616-651〕
- `_checkCompaction(assistantMessage, skipAbortedCheck = true, toolResults = [])` 的早退：`settings.enabled` 为假 → false；`skipAbortedCheck` 为真且消息的 `stopReason` 是 "aborted" → false；之后才做上下文窗口与溢出判断。〔packages/coding-agent/src/core/agent-session.ts:2599-2612〕
- 触发点在助手消息完成之后：调用 `_checkCompaction` 时传 `message`、`true` 与该轮的 `toolResults`，返回真则本次 run 直接返回 `_agentRunAbortRequested` 取反——即**一旦真的发生压缩，本轮 run 就结束**（是否算成功的 abort 由该标志决定）。〔packages/coding-agent/src/core/agent-session.ts:1516-1524〕
- 串行/并发的选择：`config.toolExecution === "sequential"`，**或**任一 toolCall 对应工具有 `executionMode === "sequential"` 时走 `executeToolCallsSequential`，否则走 `executeToolCallsParallel`。〔packages/agent/src/agent-loop.ts:505-520〕
- 取消传播：并发路径也收 `signal` 并把它传给 `prepareToolCall` 与 `executePreparedToolCall`；准备与执行之间有**两处** `signal?.aborted` 检查（取消后不再执行该工具）。〔packages/agent/src/agent-loop.ts:583-640〕
- `AgentSession` 用 getter 暴露这两个模式：`get steeringMode()`（`"all" | "one-at-a-time"`）与 `get followUpMode()`，值来自 `this.agent` 上的同名属性。〔packages/coding-agent/src/core/agent-session.ts:1308-1315〕
- 启动时从 settings 初始化：`this.agent.steeringMode = this.settingsManager.getSteeringMode()`、`this.agent.followUpMode = this.settingsManager.getFollowUpMode()`——即**可按 settings 配置**；运行中也可直接赋值。〔packages/coding-agent/src/core/agent-session.ts:2333-2351〕
