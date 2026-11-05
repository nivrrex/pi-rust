# spec/a-types.md — 消息、内容、StopReason、Usage、Tool 类型

来源以上游 `packages/*/src` 为准。
本卡只抽"类型形状"，不含 agent loop（见 `spec/b-agent-loop.md`）与 session 落盘格式（见 `spec/c-session.md`）。

## 1. 消息

- `Message` 是四个成员的联合：`SystemMessage | UserMessage | AssistantMessage | ToolResultMessage`，没有其他成员。〔packages/ai/src/types.ts:553-553〕
- `SystemMessage.role` 固定为 `"system"`，必带 `timestamp`（Unix 毫秒）；`content` 是 `string | TextContent[]`。〔packages/ai/src/types.ts:491-507〕
- `SystemMessage` 上另有三个可选字段改变"从该点起"的提示与工具集：`sections`（`Record<string, string | null>`，`null` 表示删除该 section）、`toolsAdded`（`Tool[]`）、`toolsRemoved`（`ToolReference[]`）。〔packages/ai/src/types.ts:501-505〕
- 语义：transcript 里第一条 system message 就是系统提示；之后的 system message 用 `sections` 追加或改写具名 section、用 `toolsAdded`/`toolsRemoved` 改变工具集；按顺序重放全部 system message 即可得到当前的提示与工具集。〔packages/ai/src/types.ts:481-490〕
- `UserMessage.role` 固定为 `"user"`，`content` 是 `string | (TextContent | ImageContent)[]`，必带 `timestamp`。〔packages/ai/src/types.ts:509-513〕
- `AssistantMessage.content` 是 `(TextContent | ThinkingContent | ToolCall)[]`，即助手回复里不可能出现 `ImageContent`。〔packages/ai/src/types.ts:515-517〕
- `AssistantMessage` 必带 `api`、`provider`、`model`、`usage`、`stopReason`、`timestamp`；可选 `responseModel`（provider 回报的实际模型）、`responseId`、`providerThinkingLevel`、`diagnostics`、`deferred`、`errorMessage`、`rawStopReason`（provider 原始停止原因）、`endTurn`（provider 是否显式结束回合，当前不影响控制流）。〔packages/ai/src/types.ts:518-536〕
- `ToolResultMessage<TDetails>` 是条件类型：`details` 为 JSON 兼容值时该联合成员才存在，否则整个类型退化为 `never`。必带 `toolCallId`、`toolName`、`content`（`(TextContent | ImageContent)[]`）、`isError`、`timestamp`；可选 `details`、`usage`（工具自身用量，不计入主 LLM 上下文统计）。〔packages/ai/src/types.ts:539-551〕

## 2. 内容块（Content）

- `TextContent` 的判别字段是 `type: "text"`，必带 `text`；可选 `textSignature`，注释说明它承载 OpenAI Responses 的 message metadata（legacy id 字符串或 `TextSignatureV1` 的 JSON）。〔packages/ai/src/types.ts:364-368〕
- `TextSignatureV1` 形状为 `{ v: 1; id: string; phase?: "commentary" | "final_answer" }`。〔packages/ai/src/types.ts:358-362〕
- `ThinkingContent` 的判别字段是 `type: "thinking"`，必带 `thinking`；可选 `thinkingSignature`（provider 专用的不透明/序列化 reasoning 回放数据）与 `redacted`（被安全过滤抹除时为 true，此时不透明密文存在 `thinkingSignature` 里以便回传保持多轮连续性）。〔packages/ai/src/types.ts:370-378〕
- `ImageContent` 的判别字段是 `type: "image"`，必带 `data`（base64 编码）与 `mimeType`（如 `"image/jpeg"`、`"image/png"`）。〔packages/ai/src/types.ts:380-384〕

## 3. ToolCall

- `ToolCall` 的判别字段是 `type: "toolCall"`，必带 `id`、`name`、`arguments`（`JsonObject`）；可选 `thoughtSignature`（Google 专用）与 `namespace`（OpenAI Responses 里给动态加载/带命名空间的工具用）。〔packages/ai/src/types.ts:386-394〕
- `JsonObject` 定义为 `{ [key: string]: JsonValue }`，`JsonValue` 是 `null | boolean | number | string | readonly JsonValue[] | JsonObject` 的递归联合。〔packages/ai/src/types.ts:421-422〕

## 4. StopReason

- `StopReason` 共七个值：`"pending" | "stop" | "length" | "toolUse" | "error" | "aborted" | "deferred"`。〔packages/ai/src/types.ts:419-419〕
- 流事件里对停止原因做了收窄：`done` 事件的 `reason` 只允许 `"stop" | "length" | "toolUse" | "deferred"`，`error` 事件的 `reason` 只允许 `"aborted" | "error"`；即 `"pending"` 不出现在任何流事件上。〔packages/ai/src/types.ts:663-668〕
- 流事件协议：`start` 在任何增量之前，`done` 收尾；请求建立前失败可直接以 `error` 终止，`start` 之后的失败同样以 `error` 终止；`done`/`*_delta` 不得早于 `start`。〔packages/ai/src/types.ts:636-643〕
- 直接调用 `streamSimple()` 时若缺认证会**同步抛异常**；一旦返回流，请求/模型/运行时的失败都必须编码进流里，而不是抛出。〔packages/ai/src/types.ts:341-345〕

## 5. Usage 与 cost

- `Usage` 必带 `input`、`output`、`cacheRead`、`cacheWrite`、`totalTokens` 与 `cost`；可选 `cacheWrite1h`（`cacheWrite` 中按 1h 保留写入的子集，只有 Anthropic 报这个拆分）与 `reasoning`（是 `output` 的子集，`output` 已包含它；provider 不暴露时保持 `undefined`）。〔packages/ai/src/types.ts:396-409〕
- `Usage` 里名为 `cost` 的子对象，形状是五个数字 `input`、`output`、`cacheRead`、`cacheWrite`、`total`。〔packages/ai/src/types.ts:410-417〕
- `ModelCostRates` 四项费率单位都是美元/百万 token；`ModelCostTier` 在此基础上加 `inputTokensAbove`（阈值）；`ModelCost` 加可选 `tiers` 数组。〔packages/ai/src/types.ts:941-956〕
- `calculateCost(model, usage)` 的算法：先把 `usage.input + cacheRead + cacheWrite` 当作 input token 总量，在 `model.cost.tiers` 里选出 `inputTokensAbove` 最大且小于该总量的那一档费率（没有则用 `model.cost`），然后按 `/1000000` 折算四项。〔packages/ai/src/models.ts:900-916〕
- `cacheWrite` 的计价对 Anthropic 长保留做 2 倍处理：`rates.cacheWrite * shortWrite + rates.input * 2 * longWrite`，其中 `longWrite = usage.cacheWrite1h ?? 0`，`shortWrite = usage.cacheWrite - longWrite`；`total` 是四项之和。〔packages/ai/src/models.ts:911-919〕
- `calculateCost` 是**原地改写** `usage.cost` 并返回它（形参是 `usage: Usage` 而非 `&` 之外的副本语义由调用方共享），调用方不应假设传入对象的 `cost` 保持原值。〔packages/ai/src/models.ts:900-900〕

## 6. Tool 与 ToolResult

- `Tool<TParameters>` 必带 `name`、`description`、`parameters`（typebox 的 `TSchema`），可选 `constrainedSampling`（`false` 或 `ConstrainedSamplingConfig`）。〔packages/ai/src/types.ts:600-605〕
- `ConstrainedSamplingConfig` 只有两种形态：`{ type: "json_schema"; strict: "prefer" | "require" }` 与 `{ type: "grammar"; variants: GrammarVariants }`。〔packages/ai/src/types.ts:590-598〕
- `ToolReference` 只有 `name` 一个字段，用于 `SystemMessage.toolsRemoved`。〔packages/ai/src/types.ts:607-609〕
- 运行时工具结果类型是 `AgentToolResult<T>`：必带 `content`（`(TextContent | ImageContent)[]`）与 `details`，可选 `usage`（工具自身用量，不用于主 LLM 上下文统计）与 `terminate`。〔packages/agent/src/types.ts:419-432〕
- `terminate` 的语义是"提示 agent 在当前这批工具之后停止"，且**只有当该批次里每一个已finalize 的工具结果都为 true 时才会真的提前终止**。〔packages/agent/src/types.ts:427-431〕
- 工具向 UI 流式回传部分结果用 `AgentToolUpdateCallback<T>`，其作用域限于当前这次 `execute()` 调用，工具 promise settle 之后的调用被忽略。〔packages/agent/src/types.ts:434-440〕
- `AgentTool` 在 `Tool` 之上追加 `label` 等 UI 字段，属于运行时定义，不在 pi-ai 的类型面里。〔packages/agent/src/types.ts:442-445〕

## 7. 请求上下文

- 公共入口接受的是 `Context`（`systemPrompt?`、`messages`、`tools?`），其中 `systemPrompt` 与 `tools` 只是首条 system message 的简写。〔packages/ai/src/types.ts:611-621〕
- provider 实际收到的是 `TranscriptContext`：只有 `messages` 加一个 brand 字段，只有 `normalizeContext()` 能产出该类型，裸 `Context` 无法意外进入 provider 代码。〔packages/ai/src/types.ts:623-634〕
- `StreamFunction` 的契约注释写明：收到的是规范化后的 transcript，系统提示与工具声明位于首条 system message，绝不在 context 本体上。〔packages/ai/src/types.ts:335-345〕

## 8. 流事件

- `AssistantMessageEvent` 的成员共 12 个：`start`、`text_start`、`text_delta`、`text_end`、`thinking_start`、`thinking_delta`、`thinking_end`、`toolcall_start`、`toolcall_delta`、`toolcall_end`、`done`、`error`。〔packages/ai/src/types.ts:652-668〕
- 每个增量事件都带 `contentIndex` 与 `partial`（当前累计的 `AssistantMessage`）；`toolcall_end` 额外带完整的 `toolCall`。〔packages/ai/src/types.ts:654-662〕
- `partial` 是"到目前为止的实时响应"辅助对象，不是事件时刻的快照：text 与 thinking 块在各自 `*_start` 时为空，只通过对应 `*_delta` 增长，直到权威的 `*_end`。〔packages/ai/src/types.ts:645-650〕
- 被抹除的 thinking（redacted）可能在 `thinking_start` 时就已完整且不再发 delta；`toolcall_start` 处的工具参数是 provider 专有的，`toolcall_delta` 携带后续 JSON 更新。〔packages/ai/src/types.ts:648-650〕

## 9. 图片与延迟句柄

- `AssistantImages` 带 `api`、`provider`、`model`、`output`、`stopReason`、`timestamp`，可选 `responseId`、`usage`、`errorMessage`；`ImagesStopReason` 只有 `"stop" | "error" | "aborted"`。〔packages/ai/src/types.ts:562-574〕
- `DeferredHandle` 带 `provider`、`modelId`、`api`、`id`（provider token，如 response id 或 batch id + row id），可选 `expiresAt`、`pollAfterMs`、`data`（重建最终 assistant message 所需的 provider 转换数据）。〔packages/ai/src/types.ts:469-479〕
- `AssistantMessage` 上的 `deferred` 字段承载该句柄，对应停止原因 `deferred`。〔packages/ai/src/types.ts:527-529〕

## 未覆盖 / 不确定

- `agent/src/harness/**` 的这批类型属**另一个包**：在 coding-agent 里只有 `experimental/mini/**` 会引用 harness（例如 `JsonlSessionRepo`、`reduceLaneSnapshot`）。〔packages/coding-agent/src/experimental/mini/server/run.ts:13-15〕
- 因此 MessageEntry、UsageRow、UsageWrite、ToolCallSource 与 pico3 这批类型对本项目（PLAN 第 0 节的四个内置工具 + 核心会话）属**“不适用”**——不是“未抽”，而是不在范围内（引用同上，核心路径只用到 `JsonlSessionRepo` 这类实验性代码）。〔packages/coding-agent/src/experimental/mini/server/run.ts:13-15〕
- `normalizeContext(context)` 把 `systemPrompt` 与 `tools` **折叠成一条前导 system 消息**：`createInitialSystemMessage(context.systemPrompt, context.tools)` 非空时前置到 `context.messages` 之前，返回 `{messages}`。〔packages/ai/src/utils/transcript.ts:30-34〕
- 该前导消息的形状：`role: "system"`、`content: systemPrompt ?? ""`、有工具时带 `toolsAdded`、`timestamp: 0`。〔packages/ai/src/utils/transcript.ts:18-22〕
- 注释与辅助函数表明它只管**最前面**那条 system：`getInitialSystemMessage` 取它、`dropInitialSystemMessage` 丢掉它（给把系统提示单独传的 API 用）；`context.messages` 里的其它消息不参与合并。〔packages/ai/src/utils/transcript.ts:25-54〕
- `AssistantMessageDiagnostic` 的字段是 `type`、`timestamp`、可选 `error`、可选 `details`。〔packages/ai/src/utils/diagnostics.ts:10-15〕
- 它由 `createAssistantMessageDiagnostic` 构造，并由 `appendAssistantMessageDiagnostic` 追加到消息上。〔packages/ai/src/utils/diagnostics.ts:34-45〕
- `DiagnosticErrorInfo` 由 `extractDiagnosticError` 抽取：非 `Error` 抛出的名字固定为 ThrownValue，消息文本由 `formatThrownValue` 给出（Error 取 message 或 name）。〔packages/ai/src/utils/diagnostics.ts:17-32〕
- `ModelImageResizeOptions` 的字段是 `maxWidth?`、`maxHeight?`、`maxBytes?`（后者是 base64 载荷字节上限）。〔packages/ai/src/types.ts:958-963〕
- 模型上的 `resize` 与 `inputLimits` 字段分别用 `ModelImageResizeOptions` 与 `ModelInputLimits` 类型；`ModelImageInputLimits` 另有 `maxPerMessage?`、`maxPerRequest?`。〔packages/ai/src/types.ts:966-996〕
- 这些限额**确实影响消息内容**：注释写明 resize 是“新图片进入会话历史前应用的 cache-safe 配置”，工具结果图片路径把它作为 `resizeOptions` 带下去。〔packages/coding-agent/src/utils/tool-result-images.ts:40-48〕
- 已验证：`rawStopReason` 落的是 provider 的原始停止原因，与 `stopReason` 不完全同名——OpenAI 的 `tool_calls` 映射成 `toolUse`、Anthropic 的 `end_turn` 映射成 `stop`，而原始值原样保留在 `rawStopReason`。〔golden/S01/session.jsonl:6-6〕〔golden/S12-anthropic/session.jsonl:6-6〕
- 已验证：oracle 跑出来的真实事件序列覆盖第 8 节的 12 个成员——OpenAI 侧出现过 `toolcall_end`，Anthropic 侧同样出现过 `toolcall_end` 与 `text_start`，10 份期望结果可逐条比对。〔fixtures/expected/openai-parallel-calls.expected.json:55-55〕〔fixtures/expected/anthropic-tool-use-delta.expected.json:45-45〕
- 已验证（**不适用于真实 provider 流**）：`deferred` 只出现在 `StopReason` 联合类型、请求选项 `deferred`、以及 `faux` 这个测试替身 provider 里；`src/providers/` 下除 `faux.ts` 外没有任何真实 provider 产出该分支——0.87.1 的真实流里不会出现它，现有夹具没有对应响应不是缺口。〔packages/ai/src/types.ts:330-419〕
  - 类型层面：`StopReason` 含 `"deferred"`；请求侧 `SimpleStreamOptions.deferred` 接受 `boolean` 或 `{ window?: "15m" | "1h" | "24h" }`，语义是"向 provider 要一个持久句柄、请求异步继续"；会话事件的 `reason` 字段只接受 `stop` / `length` / `toolUse` / `deferred` 四种。〔packages/ai/src/types.ts:330-419〕
  - 唯一实现是 `providers/faux.ts`：它会返回 `stopReason: "deferred"` 并挂一个 `DeferredHandle`，配合 `deferredFetchCount` / `cancelledDeferred` 之类的状态做手动放行，属测试替身能力。〔packages/ai/src/providers/faux.ts:82-303〕
- 保留说明：`faux` 这个测试替身属于本项目范围外（我们用 `tools/mock-provider`），但 Rust 实现**必须**保留 `StopReason` 的 `deferred` 变体以保证与原版会话文件互通——读到含该值的旧会话不能解析失败。〔packages/ai/src/types.ts:419〕

- 已验证：`anthropic-messages` 的 beta 通道固定打 `POST /v1/messages?beta=true`——`@anthropic-ai/sdk/resources/beta/messages/messages.js:43` 写死该路径；`client.js:444-449` 在 OAuth 模式下追加 `anthropic-beta` 头。同文件 `:44` 的 `timeout ?? 600000` 说明该通道默认超时 600 秒。该字段是**请求体字段**而非 beta 值，由 compat 的 `supportsEagerToolInputStreaming` 决定是否下发。〔fixtures/vendor-evidence/anthropic-sdk-beta-messages.txt:7-19〕
