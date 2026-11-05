# spec/d-compaction.md — compaction

来源 `packages/coding-agent/src/core/compaction/{compaction,utils}.ts`
与 `packages/coding-agent/src/core/settings-manager.ts`（per-model 覆盖）。
`CompactionEntry` 的字段已在 `spec/c-session.md` 第 2 节列出，本卡不重复。

## 1. 触发阈值

- 触发判据是 `shouldCompact(contextTokens, contextWindow, settings)`：`settings.enabled` 为假直接返回 false，否则判 `contextTokens > contextWindow - settings.reserveTokens`。〔packages/coding-agent/src/core/compaction/compaction.ts:289-292〕
- `CompactionSettings` 三个字段：`enabled`、`reserveTokens`、`keepRecentTokens`；默认值是 `{ enabled: true, reserveTokens: 16384, keepRecentTokens: 20000 }`。〔packages/coding-agent/src/core/compaction/compaction.ts:142-152〕
- 上下文 token 的取法：优先用 provider 报的 `totalTokens`，没有则退回 `input + output + cacheRead + cacheWrite`。〔packages/coding-agent/src/core/compaction/compaction.ts:158-164〕
- 产品层入口 `_checkCompaction` 先取 `settingsManager.getCompactionSettings(this.model)`，`enabled` 为假直接 false。〔packages/coding-agent/src/core/agent-session.ts:2599-2605〕
- `skipAbortedCheck` 默认 true，此时 `stopReason === "aborted"` 的消息直接跳过压缩。〔packages/coding-agent/src/core/agent-session.ts:2601-2608〕
- 若该 assistant 消息来自**另一个模型**，跳过 overflow 检查；判据是 `sameModel`（provider 与 model 都要等于当前模型），注释理由是用户可能刚从 opus 切到 codex，旧模型的 overflow 错误不该触发新模型的压缩。〔packages/coding-agent/src/core/agent-session.ts:2612-2617〕
- 若该消息的时间戳早于等于最近一次 compaction entry 的时间戳则直接跳过，判据是 `assistantIsFromBeforeCompaction`（用 `getLatestCompactionEntry(this.sessionManager.getBranch())` 求边界）；注释理由是防止压缩后第一条 prompt 被过期的压缩前 usage/error 重新触发压缩。〔packages/coding-agent/src/core/agent-session.ts:2619-2627〕
- 压缩后写入新 `CompactionEntry`，再 `_refreshFinalizedContext()`，并用 `estimateMessagesTokens` 估算压缩后的 token 数写进结果对象的 `estimatedTokensAfter`。〔packages/coding-agent/src/core/agent-session.ts:2499-2502〕〔packages/coding-agent/src/core/agent-session.ts:2519-2526〕

## 2. per-model 覆盖

- settings 里的形状是 `compaction.modelOverrides?: Record<string, CompactionModelOverride>`，键是精确的 `"provider/modelId"`。〔packages/coding-agent/src/core/settings-manager.ts:23-28〕
- 取值优先级是 `override ?? ordinary ?? DEFAULT_COMPACTION_TOKEN_SETTINGS[field]`，即模型覆盖 → 普通设置 → 内置默认，三者逐字段独立回退。〔packages/coding-agent/src/core/settings-manager.ts:864-884〕
- 校验在 `getCompactionTokenSetting` 里：普通值 `ordinary` 不是非负安全整数就抛 `Expected a non-negative safe integer.`；模型覆盖项 `entry` 不是可合并对象就抛 `Expected an object.`；覆盖值 `override` 同样要非负安全整数。〔packages/coding-agent/src/core/settings-manager.ts:859-883〕
- 即使当前模型有合法覆盖，普通设置里的非法值仍然报错（只有**缺失**的普通字段 `ordinary` 才用 `DEFAULT_COMPACTION_TOKEN_SETTINGS[field]`）。〔packages/coding-agent/src/core/settings-manager.ts:864-884〕
- `enabled` 是全局的，不参与 per-model 覆盖。〔packages/coding-agent/src/core/settings-manager.ts:23-26〕

## 3. cut point 规则

- 合法切点集合 `findValidCutPoints` 遍历 `[startIndex, endIndex)`，跳过 `type === "compaction"` 的条目，其余只要该 entry 的上下文消息里**有一个**满足 `isCutPointMessage` 就把下标加入。〔packages/coding-agent/src/core/compaction/compaction.ts:416-428〕
- `isCutPointMessage` 允许的角色是 `user`、`assistant`、`bashExecution`、`custom`、`branchSummary`、`compactionSummary`；`toolResult` 明确返回 false（工具结果必须跟着它的工具调用）。〔packages/coding-agent/src/core/compaction/compaction.ts:373-386〕
- 算法本体：从最新往回累加 `estimateTokens`，达到 `keepRecentTokens` 就在该处切断；默认切点是最早的合法切点（保留下文说的"从第一条消息开始"）。〔packages/coding-agent/src/core/compaction/compaction.ts:480-501〕
- 切点选择是取"当前 entry 位置或其之后"的最靠前合法切点，即源码里的 `cutPoints.find((candidate) => candidate >= i)`；找不到就退回 `cutPoints[cutPoints.length - 1]`，注释解释是为了处理尾部工具结果本身就超预算的情况。〔packages/coding-agent/src/core/compaction/compaction.ts:493-500〕
- 切点确定后再向前回扫，把**不影响上下文**的相邻元数据 entry 并入保留区间；遇到 `prevEntry.type === "compaction"` 或 `sessionEntryToContextMessages(prevEntry).length > 0` 就停。〔packages/coding-agent/src/core/compaction/compaction.ts:503-511〕
- 没有任何合法切点时返回 `{ firstKeptEntryIndex: startIndex, turnStartIndex: -1, isSplitTurn: false }`。〔packages/coding-agent/src/core/compaction/compaction.ts:474-478〕
- `CutPointResult` 三字段：`firstKeptEntryIndex`（开始保留的 entry 下标）、`turnStartIndex`（被切开的那个 turn 的起始 user 消息下标，不切时为 -1）、`isSplitTurn`（切点是否不是 turn 起始）。〔packages/coding-agent/src/core/compaction/compaction.ts:443-450〕
- 注释里的切点约束：可以在 user 或 assistant 消息处切，但**永远不能在工具结果处切**；切在带工具调用（`tool calls`）的 assistant 消息上时，它的工具结果在后面会被一起保留。〔packages/coding-agent/src/core/compaction/compaction.ts:452-466〕

## 4. turn 边界与 split-turn

- `isTurnStartMessage` 与 `isCutPointMessage` 的差别只在 `assistant`：assistant 是合法切点但**不是** turn 起点。〔packages/coding-agent/src/core/compaction/compaction.ts:388-401〕
- `isTurnStartEntry` 对 `type === "compaction"` 直接返回 false，其余看该 entry 的上下文消息里有没有 turn 起点角色。〔packages/coding-agent/src/core/compaction/compaction.ts:403-408〕
- `findTurnStartIndex` 从给定下标往回找第一个 turn 起点 entry，找不到返回 -1。〔packages/coding-agent/src/core/compaction/compaction.ts:434-441〕
- 判定式在源码里写作 `isSplitTurn: !startsTurn && turnStartIndex !== -1`：切点不在 turn 起点、且能往回找到 turn 起点，才算切开了一个 turn。〔packages/coding-agent/src/core/compaction/compaction.ts:513-521〕
- 切在 assistant 消息上时，被切开的 turn 是从它前面那个 user 消息开始的，所以要把 `turnPrefixMessages` 单独摘要。〔packages/coding-agent/src/core/compaction/compaction.ts:925-934〕

## 5. 准备阶段与保留策略

- `prepareCompaction` 的第一步短路：路径最后一条已经是 compaction（`pathEntries[pathEntries.length - 1].type === "compaction"`）就返回 undefined，不重复压缩。〔packages/coding-agent/src/core/compaction/compaction.ts:894-900〕
- 准备阶段先 `buildSessionProjection(pathEntries)` 拿投影，用 `entry.sourceEntry.type === "compaction" && entry.messages.length > 0` 找**最新的有效** compaction 下标，注释说明更旧的 compaction 仍可能落在它的保留原始区间里但投影贡献为空。〔packages/coding-agent/src/core/compaction/compaction.ts:902-909〕
- 保留边界 `boundaryStart` = 上一次 compaction 下标 + 1；`previousSummary` 取那条 compaction 的 `summary`；注释强调规范投影已经选好了上一次压缩保留的尾巴，所以直接从下一条开始。〔packages/coding-agent/src/core/compaction/compaction.ts:911-917〕
- `tokensBefore` 用 `estimateProjectedContextTokens(projection, pathEntries).tokens`，即**从重建后的、已应用 context edit 的投影**重新计算，而不是复用上一条 compaction 的数字。〔packages/coding-agent/src/core/compaction/compaction.ts:918-920〕
- 已验证：除阈值触发外，**context overflow 错误**同样进入压缩，事件里 `compaction_start` 的 `reason` 是 `overflow`，随后发摘要请求并重试原请求（本次录制 mock 共收到 4 个请求）。〔golden/S13c/events.jsonl:15-15〕〔golden/S13c/session.jsonl:7-7〕
- 已验证：`tokensBefore` 写的是**上一次 assistant 的 usage**（25000），而不是按字符估算的值——同一条消息的正文只有十几个字符。〔golden/S19/session.jsonl:7-7〕
- 已验证：会话过小时压缩**静默不发生**——这份失败的录制只有 6 行，最后一条 assistant 以 `stopReason` 收尾，全程没有任何 compaction 条目，mock 也只收到一个请求（该结论是"缺失"类证据，检查器只能验到 stopReason 那一行）。〔golden/_rejected/S19-attempt-3/session.jsonl:6-6〕
- 已验证：`isSplitTurn` 为真时摘要按 `历史摘要 + 分隔 + Turn Context` 合并，本次录制里历史摘要为空，内容以 `No prior history.` 开头，随后是 `**Turn Context (split turn):**`。〔golden/S21/session.jsonl:9-9〕
- 要摘要的历史区间是 `messagesToSummarize`，按 `slice(boundaryStart, historyEnd)` 取，其中 `historyEnd` 在 split turn 时是 `cutPoint.turnStartIndex`、否则是 `cutPoint.firstKeptEntryIndex`；`turnPrefixMessages` 取 `slice(cutPoint.turnStartIndex, cutPoint.firstKeptEntryIndex)`，仅在 `isSplitTurn` 时非空。〔packages/coding-agent/src/core/compaction/compaction.ts:925-934〕
- 两个区间都空（`messagesToSummarize.length === 0 && turnPrefixMessages.length === 0`）就返回 undefined，无事可做。〔packages/coding-agent/src/core/compaction/compaction.ts:936-936〕
- 文件操作从被编辑过的、模型可见的消息里经 `extractFileOperations` 抽取，split turn 时**额外**用 `extractFileOpsFromMessage` 从 turn 前缀消息里抽。〔packages/coding-agent/src/core/compaction/compaction.ts:938-946〕
- `CompactionPreparation` 的返回字段：`firstKeptEntryId`、`messagesToSummarize`、`turnPrefixMessages`、`isSplitTurn`、`tokensBefore`、`previousSummary`、`fileOps`、`settings`。〔packages/coding-agent/src/core/compaction/compaction.ts:948-957〕
- retain 策略的落点是 `firstKeptEntryId`：注释规定上下文跟随当前叶子路径，若路径里有 compaction 条目，则由最新的 compaction 条目本身充当摘要，后面接从 `firstKeptEntryId` 开始的保留条目与该 compaction 之后的所有条目，更早被摘要的条目被省略。〔packages/coding-agent/src/core/session-manager.ts:470-474〕
- "retain-none" 压缩（把自己的 id 记为 `firstKeptEntryId`，使下一次压缩从它之后开始）在 0.87.1 的 src 里没有独立分支，本次未在源码中定位到产生该情形的代码路径，仅在文档中描述。**待验证**〔packages/coding-agent/docs/compaction.md:83-83〕
- 被 `context_edit` 省略掉的原始条目仍然存储（`Omitted raw entries remain stored`），但不影响切点选择、摘要、checkpoint 与 token 估算。〔packages/coding-agent/docs/compaction.md:83-83〕

## 6. 摘要请求的构造

- 摘要是**独立的一次请求**：`buildSummarizationContext(promptText)` 用 `normalizeContext({ systemPrompt: SUMMARIZATION_SYSTEM_PROMPT, messages: [单条 user text 消息] })` 构造 `TranscriptContext`。〔packages/coding-agent/src/core/compaction/compaction.ts:703-715〕
- 系统提示是常量 `SUMMARIZATION_SYSTEM_PROMPT`，定义在 `core/compaction/utils.ts`，逐字内容需在实现压缩时照抄。〔packages/coding-agent/src/core/compaction/utils.ts:156-156〕
- `completeSummarization` 强制 `cacheRetention: "none"`，注释理由是"一次性摘要不要写缓存"；`sessionId` 复用调用方给的，没有就给一个新的 `uuidv7()`（分支摘要属于这类，会拿到全新 routing id）。〔packages/coding-agent/src/core/compaction/compaction.ts:649-655〕
- 重试落在这一层：`completeSummarization` 用 `retryAssistantCall(produce, retry, requestOptions.signal, callbacks)` 包住，注释指明这是为了容忍瞬时流中断。〔packages/coding-agent/src/core/compaction/compaction.ts:636-660〕
- split turn 时生成**两份**摘要并合并：`historyText`（无历史时用字面量 `No prior history.`）+ `\n\n---\n\n**Turn Context (split turn):**\n\n` + turn 前缀摘要。〔packages/coding-agent/src/core/compaction/compaction.ts:1016-1055〕〔golden/S21/session.jsonl:9-9〕
- 合并后的 usage：历史摘要成功时用 `combineUsage(historyUsage, turnPrefixResult.usage)`，否则只用 turn 前缀那份。〔packages/coding-agent/src/core/compaction/compaction.ts:1055-1055〕
- 非 split turn 时只生成一份历史摘要（`else` 分支直接调 `generateSummaryWithUsage(messagesToSummarize, ...)`）。〔packages/coding-agent/src/core/compaction/compaction.ts:1056-1065〕
- `generateSummary` 只是 `generateSummaryWithUsage(...).text` 的薄封装。〔packages/coding-agent/src/core/compaction/compaction.ts:667-701〕
- 文件操作在摘要里以 XML 标签形式给出：`<read-files>` 与 `<modified-files>`，两段都空时返回空字符串。〔packages/coding-agent/src/core/compaction/utils.ts:69-80〕
- `computeFileLists` 把 edited 与 written 的并集当 modified，只被 read 过且未被修改的路径进 readFiles，两者都 `.sort()` 排序。〔packages/coding-agent/src/core/compaction/utils.ts:62-67〕

## 7. CompactionEntry 的写入路径

- 手动压缩与自动压缩走同一个 `sessionManager.appendCompaction(summary, firstKeptEntryId, tokensBefore, details, fromExtension, usage)`；扩展提供了 compaction 内容时直接用扩展给的字段，否则用默认摘要生成器（两种路径共用）。〔packages/coding-agent/src/core/agent-session.ts:2469-2499〕
- 压缩被 abort 时抛 `Compaction cancelled`。〔packages/coding-agent/src/core/agent-session.ts:2495-2497〕
- 压缩完成后向扩展发 `session_compact` 事件，带 `compactionEntry`、`fromExtension`、`reason: "manual"`、`willRetry: false`。〔packages/coding-agent/src/core/agent-session.ts:2509-2517〕
- 对外发 `compaction_end` 前先 `_clearManualCompactionState()`，注释理由是 `compaction_end` 监听器可能提交排队中的 prompt，必须先暴露 idle 状态。〔packages/coding-agent/src/core/agent-session.ts:2527-2530〕

## 未覆盖 / 不确定

- 已验证：估算统一按**字符数**做，`CHARS_PER_TOKEN = 4`、向上取整；图片按常量 `ESTIMATED_IMAGE_CHARS = 4800` 字符折算。〔packages/ai/src/utils/estimate.ts:15-42〕
  - `estimateTextTokens(text) = ceil(text.length / 4)`；`estimateTextAndImageContentTokens` 先按块累加字符（`text` 取 `text.length`，图片取 4800），再除 4 向上取整；content 为**字符串**时直接取其长度。〔packages/ai/src/utils/estimate.ts:29-42〕
  - `estimateMessageTokens` 按 role 分派：`system` = 系统文本加 `toolsAdded`/`toolsRemoved` 的工具定义估算；`user` / `toolResult` = 文本图片估算；`assistant` 逐块累加 `text.length` + `thinking.length` + `toolCall` 的 `name.length` 与 `JSON.stringify(arguments).length`（序列化抛错时用 `"[unserializable]"`，返回 `undefined` 时用 `"undefined"`）。〔packages/ai/src/utils/estimate.ts:44-69〕
  - `calculateContextTokens` 的口径是 `usage.totalTokens`，为 0 时退回 `input + output + cacheRead + cacheWrite`。〔packages/ai/src/utils/estimate.ts:18-20〕
  - `estimateContextTokens` 优先用**真实 usage**：`getLastAssistantUsageInfo` 从后往前找最后一条 assistant，要求其 `timestamp` 不早于其后所有消息的最大 timestamp（即 usage 之后没有插入压缩摘要之类的新前缀消息），且 `stopReason` 不是 `aborted`/`error`、且 `calculateContextTokens > 0`；找到就用该 usage 加其后所有消息的估算值，没有才逐条累加。〔packages/ai/src/utils/estimate.ts:71-110〕
  - agent 侧的 `estimateTokens` 公式与之平行但更保守：图片只在 `block.type === "image"` 时计 4800，缺 `text` 的 text 块不计；assistant 的 `toolCall` 显式匹配，toolResult 与自定义块不计。〔packages/agent/src/harness/compaction/compaction.ts:251-290〕
  - `estimateProjectedContextTokens(projection, branchEntries)` 在 `estimateContextTokens` 之上把 `lastUsageIndex` 映射回 entry id（累加 `entry.messages.length` 直到跨过该下标），用于标明这条 usage 来自哪个 entry。〔packages/coding-agent/src/core/compaction/compaction.ts:249-263〕
- 分支摘要的**触发条件**是 `navigateTree`：它先 `collectEntriesForBranchSummary` 求出要摘要的条目与公共祖先，需要摘要时再调 `generateBranchSummary`。〔packages/coding-agent/src/core/agent-session.ts:3612-3680〕
- `generateBranchSummary` 的默认 `reserveTokens = 16384`，预算为 `contextWindow - reserveTokens`；被摘要内容作为 `role: "user"` 的消息经 `normalizeContext` 与 `SUMMARIZATION_SYSTEM_PROMPT` 一起提交。〔packages/coding-agent/src/core/compaction/branch-summarization.ts:293-355〕
- 定制指令拼成 `BRANCH_SUMMARY_PROMPT` 加一段 Additional focus；返回前再用 `BRANCH_SUMMARY_PREAMBLE` 作前缀。〔packages/coding-agent/src/core/compaction/branch-summarization.ts:253-370〕
- 已验证：溢出判定分三种情况。`isContextOverflow(message, contextWindow?)`：错误文案命中 `OVERFLOW_PATTERNS` 之一（且不命中 `NON_OVERFLOW_PATTERNS` 的限流等非溢出模式）即为溢出，`cerebras` 另有一条无正文溢出模式；无正文但 `stopReason === "stop"` 且 `usage.input + usage.cacheRead > contextWindow` 判为静默溢出；`stopReason === "length"` 且 `usage.output === 0` 且 `input + cacheRead >= contextWindow * 0.99` 判为长度截断溢出。〔packages/ai/src/utils/overflow.ts:136-170〕
  - `isRecoverableLength(message, desiredMaxOutput)`：`stopReason === "length"` 且 `desiredMaxOutput > 0` 且 `usage.output < desiredMaxOutput`。调用方必须传**未被上下文夹紧**的原始上限。〔packages/ai/src/utils/overflow.ts:178-180〕
  - 恢复流程：`_checkCompaction` 先过四道闸——压缩未启用直接 false；`skipAbortedCheck` 为真且 `stopReason === "aborted"` 直接 false；消息的 provider/model 与当前模型不同直接 false（换模型后旧模型的溢出不该触发压缩）；消息时间戳早于或等于最近一次压缩边界直接 false。〔packages/coding-agent/src/core/agent-session.ts:2599-2631〕
  - 随后判定 `contextOverflow` 或 `recoverableLength` 任一成立即进入恢复。两者成立时 `willRetry = stopReason !== "stop"`：`stop` 表示响应已成功完成，只压缩**不重试**（`agent.continue()` 无法从已完成的 assistant 响应续跑）；其余情况若 `_overflowRecoveryAttempted` 已置位，则发一条 `compaction_end`（`reason: "overflow"`、`willRetry: false`、带失败文案）并返回 false；否则置位该标记、**省略**本次最终尝试（不落盘），再跑 `_runAutoCompaction("overflow", willRetry)`。全流程只重试一次。〔packages/coding-agent/src/core/agent-session.ts:2664-2694〕
- 保留边界的推进规则在源码注释里写明：`recovery attempt` 及其省略编辑在最后一条可见输入之后是 context-invisible，**只有对“闭合的后缀”（其省略编辑都落在该后缀内）才推进边界**。〔packages/coding-agent/src/core/compaction/compaction.ts:853-854〕
- 实现方式：先 `const omittedSuffixIds = new Set(suffix.filter(isOmitted).map((entry) => entry.sourceEntry.id));`，再要求候选目标不在该集合内。〔packages/coding-agent/src/core/compaction/compaction.ts:861-866〕
- 该判断在 `findProjectedCutPoint` 里，由 `prepareCompaction` 以 `boundaryStart`/`boundaryEnd`/`settings.keepRecentTokens` 调用。〔packages/coding-agent/src/core/compaction/compaction.ts:824-920〕
- 在 0.87.1 的 `packages/*/src` 里搜不到 `retain-none`（也不存在 `retainNone`）：该字符串只出现在 `CHANGELOG.md`、`docs/compaction.md`、`docs/session-format.md` 与两个测试文件里——即**源码里没有产生它的独立路径**。〔packages/coding-agent/docs/compaction.md:83-83〕
- 文档描述的对应行为是“重复压缩时摘要区间从上次的 kept 边界开始”，涉及的字段 `firstKeptEntryId` 在源码里确实存在。〔packages/coding-agent/src/core/compaction/compaction.ts:106-106〕
- `SUMMARIZATION_SYSTEM_PROMPT` 定义在 `core/compaction/utils.ts`，被 compaction 与分支摘要共用（逐字内容见该行）。〔packages/coding-agent/src/core/compaction/utils.ts:156-156〕
- turn-prefix 提示词是 `TURN_PREFIX_SUMMARIZATION_PROMPT`，开头整句为 "The messages above are earlier context from an ongoing conversation."，其后要求给出 concise checkpoint 并说明它会放在后续消息之前（逐字内容见该行）。〔packages/coding-agent/src/core/compaction/compaction.ts:964-970〕
- 使用侧：`turnPrefixMessages` 由切点产生并进入摘要输入（`prepareCompaction` 一带）。〔packages/coding-agent/src/core/compaction/compaction.ts:930-951〕
- 压缩有三条独立的 `AbortController`：`_compactionAbortController`、`_autoCompactionAbortController`、`_branchSummaryAbortController`；`get isCompacting()` 只要任一存在就为真。〔packages/coding-agent/src/core/agent-session.ts:1294-1300〕
- `compact()` 先 `await this.abort()`，再 `new AbortController()` 建一个新的，并 `_emit({type: "compaction_start", reason: "manual"})`。〔packages/coding-agent/src/core/agent-session.ts:2406-2410〕
- signal 的传播链：`_getSummarizationRequestAuth(model, this._compactionAbortController.signal)` 与请求选项里的 `signal` 都用它；调用**之后**再用 `.signal.aborted` 判定结果（并与 `cancelledByExtension` 合并）。〔packages/coding-agent/src/core/agent-session.ts:2425-2539〕
- 分支摘要走自己那条：`_branchSummaryAbortController.signal`。〔packages/coding-agent/src/core/agent-session.ts:3646-3646〕
