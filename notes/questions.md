# notes/questions.md — 待决问题

## Q1（工程判断，未解决）cost 末位浮点与 TS 版不一致
- 现象：`fixtures/expected/openai-parallel-calls.expected.json` 里 `cost.input` 为 `5e-06`，而按 `packages/ai/src/models.ts` 的 `(rates.input / 1000000) * usage.input` 在 Node 与 Rust 上都算出 `4.9999999999999996e-06`；`total` 也有同量级差异（TS `1.3999999999999998e-05` vs 直接相加 `1.3999999999999999e-05`）。
- 已排除：JSON 序列化精度（`JSON.stringify` 保留完整位）、模型费率不同（oracle 传入的是固定费率 1/2/0/0）。
- 影响：只影响 usage.cost 的末位，不影响事件序、消息内容、停止原因。
- 现状处理：`crates/pi-ai/tests/g0.rs` 的 `round_costs` 把两侧 cost 取整到 1e-12 网格再比较，差异大于该量级仍会失败。
- 需要人工确认：TS 侧实际使用的算式（是否在别处又做了一次折算或重排求和顺序）。

## Q2（已关闭）录制黄金输出与 AGENTS.md 第 8 条的 `--no-session` 冲突

> 关闭依据：RUNBOOK-2 第 1 节。允许录制时不加 `--no-session`；保护来自临时目录 + 网络命名空间，AGENTS.md 第 8 条已相应改写。
- 冲突点：AGENTS.md 第 8 条要求运行任何 pi 前"使用 `--no-session`"，但 RUNBOOK-1 C2/C3 要求采集 `session.jsonl` 作为 A 层比较对象之一。
- 当前处理：只把 `PI_CODING_AGENT_DIR` / `PI_CODING_AGENT_SESSION_DIR` 指向临时目录并在整个 `unshare` 网络命名空间内运行，不加 `--no-session`；临时目录跑完即删。隔离与"不产生真实调用"的意图仍然满足。
- 需要人工确认：是否允许在录制场景下省略 `--no-session`（仅保留临时目录隔离），或者修改 AGENTS.md 第 8 条措辞。

## Q3（部分解决）自动压缩场景 S19

- 前提已验证：压缩检查用上一次 assistant 的 usage；第一次请求不可能触发压缩（`ref/.../agent-session.ts:595-605`、`compaction.ts:249-270`）。
- 阈值侧全部就绪且无效：`contextWindow=20000`、`reserveTokens=1000` 已写进 models.json/settings.json，夹具 usage 25000 > 19000，`shouldCompact` 判据成立（`compaction.ts:289-292`）。
- 已尝试四种办法（均未产生 compaction entry、未产生摘要请求）：
  1. 两次运行 + `--continue`（followUp），但 keepRecentTokens=2000 → cut point 回退到第一条，`messagesToSummarize` 为空，`prepareCompaction` 返回 undefined；
  2. 同上但 keepRecentTokens 降到 50 → 仍无压缩；
  3. rpc 单次运行内两条 prompt（连发 / 事件驱动顺序）→ mock 只收到 1 个请求；
  4. 预置历史会话文件（`sessionFiles`）+ `--continue` 单次运行 → mock 只收到 1 个请求。
- 共同症状：mock 始终只收到 1 个请求，说明第二次/摘要请求根本没发出。
- **已解阻（见 notes/decisions.md 同批）**：根因是会话过小导致 `prepareCompaction` 返回 undefined（可摘要区间为空），不是阈值或配置问题。加大首轮回复体量后 S19 触发成功，`tokensBefore` 证明走的是 usage 估算。
- **S20 已解决**（见 notes/decisions.md）：判据必须用 `get_state` 的 `isStreaming`，不能用 `agent_end`。
- 仍未解决的是 **S21**（split-turn 构造）与由此连带的 S22：大 usage + 大工具结果时 toolResult 未落盘、助手消息未进入下一次请求的投影。
- 归档：三次失败产物在 `golden/_rejected/S19-attempt-3/`，方案 4 的产物在 `golden/S19/`（同样不可作基线，`meta.json` 的 A 层为 false）。

## Q4（已关闭）读取大文件时 toolResult 不落盘

- 现象：`scenarios/S21.json` 读 `big.txt`（约 36 KB / 1200 行）时，`golden/S21/session.jsonl` 里**没有任何 toolResult entry**；换成读小文件 `input.txt` 时，toolResult 正常落盘（内容是 ENOENT 报错，64 字符）。
- 同一场景下压缩本身是成功的（compaction entry 存在，`tokensBefore: 25000`，且 `isSplitTurn` 为真）。
- 影响：`spec/e-tools.md` 的 read 截断行为（2000 行 / 50 KB 上限）目前只有小文件证据；"大工具结果如何进入 session 与下一次请求"没有证据。S21 因此改为用文本 turn 触发 split-turn，描述已按实际观测更正。
- 需要：无需再查；已由 `golden/S21/session.jsonl` 与 `golden/S25-openai/session.jsonl` 证实。

## Q5（已关闭）rpc 的 `prompt` 不展开 slash 命令

- 现象：`scenarios/S24.json` 里 `{"type":"prompt","message":"/review-helper"}` 原样进了请求体；同一场景的 `get_commands` 却能列出 `skill:review-helper` 与 `change-summary`。
- 影响：`spec/i-cli.md` 里 slash 命令、skills、prompt template 的展开规则目前无法用 rpc 场景取证；`spec/b-agent-loop.md` 提到的 slash 命令也缺证据。
- **关闭依据（RUNBOOK-3 A2）**：之前用的是错名字。`get_commands` 返回的是 `skill:review-helper`，正确调用形式是 `/skill:review-helper <参数>`；用对名字后 print 与 rpc 两条通道都展开，skill 正文进入请求体，Q5 关闭。

## Q6（已关闭）OpenAI 格式下读图片不产生 toolResult

- 现象：`scenarios/S25-openai.json` 与 `scenarios/S02.json` 的差别只在被读文件（png vs 文本）与夹具；前者 session 里没有 `toolResult` entry，且 `--continue` 的下一次请求里上一轮 assistant 消息消失。
- 对照：`S25-anthropic` 用同一张 png 正常产生 `['text','image']` 的 toolResult；`S02`（OpenAI 格式读文本文件）也正常。
- **根因（RUNBOOK-3 A1）**：是我生成夹具时的缺陷——`render()` 期望每个元素是一「块」（字符串列表），我却把 tool_calls 行作为**裸字符串**传入，`extend` 把它逐字符拆散，于是夹具里根本没有 tool call 块。`tool_execution_start` 计数为 0 正是该症状。修正夹具后重录，`toolResult` 正常出现（约 2 万字符的截断文本 / `['text','image']`）。**不是 TS 行为**，spec 中相关限制说明已撤回。
- 需要：查 `packages/coding-agent/src/utils/image-process.ts` 与 OpenAI 适配器在 toolResult 序列化时对 image 块的分支，确认是抛错、被过滤，还是走了 `wrapToolDefinition` 的某条转换。

## Q7（已关闭）：usage 口径的两处实质差异

- 结论：`input = max(0, prompt_tokens - cacheRead - cacheWrite)`（TS 不把 prompt_tokens 直接当 input）；`totalTokens` 始终自行求和，不采信 provider 的 `total_tokens`。- 证据：`ref/pi-0.87.1/packages/ai/src/api/openai-completions.ts:1522-1547`。
- 处置：解码器已按此改写，两个回归测试钉住（含夹到 0 的边界）。详见 `notes/decisions.md`。


## Q8（已关闭）：`models.json` 读后写回只能做到语义等价

- 现状：`crates/pi-ai/tests/t2_6_models_json.rs` 比较的是 `serde_json::to_value` 的结果，只能证明**语义等价**；之前文档与测试注释里写的"逐字节等价"是错的，已更正。
- 原因：`serde_json` 未启用 `preserve_order`，对象键会被重排；启用它会引入 `indexmap` 依赖。
- 处置约定：启用 `preserve_order` 属新增依赖，按 AGENTS 须先在 `notes/questions.md` 记录、P4 之前确认。**在确认之前，任何文档都不得写"逐字节等价"。**
- 影响面：目前只有 models.json 一处。若后续 session 文件也要求字节级等价，需一并处理。

## Q9（已批准，2026-10-XX）：为 overflow 模式匹配引入 `regex` 依赖

- 背景：B4 要求实现 `isContextOverflow`。其模式表（`ref/pi-0.87.1/packages/ai/src/utils/overflow.ts:9-63`）共 25 条正则，用到 `\d`、`[\d,]+`、`(?:...)`、非贪婪 `\s*` 与 `^` 锚点。`NON_OVERFLOW_PATTERNS` 另有 3 条。
- 现状：`pi-ai::provider` 里已有一个手写的小匹配器，**只支持**字面字符、`.`、`?`、`[x]` 单字符类——是 T2.1 为 `retry.ts` 的错误模式表写的。把它扩成支持上述构造，等于自己实现正则引擎，违反 AGENTS「禁止重复实现已有成熟能力」。
- **决定**：已获批准新增 `regex` 依赖，实现见 `crates/pi-ai/src/overflow.rs`。模式表逐字来自 `overflow.ts:9-79`，用 `regex` 编译。
- 我的倾向：允许新增 `regex`。理由有三：模式表是 TS 的规格，逐字对齐才能保证差分一致；手写 25 段样板必然出现语义偏差，且新增 provider 文案时容易漏改；`regex` 是成熟、低依赖、无 unsafe 的库。
- 影响面：不加则 B4 的 S13c 无法验收，overflow 识别会长期缺席；`retry.ts` 的错误分类目前用的是自己的小匹配器，改用 `regex` 后那部分也可以换成逐字对齐的正则。

## Q10（已裁定并落地）：Anthropic 请求路径带 `?beta=true`

- 事实：oracle 捕获到 TS 打的是 `POST /v1/messages?beta=true`（10 种形态全部如此）。
- 现状：Rust 侧 `AnthropicMessagesClient` 打的是 `POST /v1/messages`。
- 未验证：`beta=true` 在 0.87.1 里代表什么能力（思考、流式工具输入等），以及不带它会不会被真实端点拒绝。源码里没找到该标志的语义说明。
- **裁定：选 B，Rust 侧加上 `?beta=true`**，与 TS 的线上行为一致。
- **依据（SDK 源码，不是照抄表象）**：`@anthropic-ai/sdk/resources/beta/messages/messages.js:43` 固定发 `POST /v1/messages?beta=true`；`client.js:444-449` 在 OAuth 模式下追加 `anthropic-beta` 头。beta 接口是 SDK 的一等公民，TS 走的正是它。
- **附带查到一条**：同文件 `:44` 有 `timeout: timeout ?? 600000`——SDK 默认超时 **600 秒**，这为此前"300s 是工程判断"提供了参照。
- **`eager_input_streaming` 不是 beta 值**，它是请求体里的字段，由 compat 的 `supportsEagerToolInputStreaming` 控制是否下发（`packages/ai/src/types.ts:780-781`、`api/anthropic-messages.ts:1486`）。

## Q11（已裁定并执行）：golden 基线的归一化形态不一致

- 现象：`golden/*/session.jsonl` 里 **12 个场景**的 assistant 消息带 `"responseId": "<PROVIDER_ID>"`（归一化器改动前录的），其余场景该键已被整键删除。D3 抽样回归显示 S01/S02 的 A 层差异**全部**来自这里（去掉该字段后逐字节一致）。
- 影响：P3 的验收（S01–S13 差分）要拿 golden 作基线；基线形态不统一会让每个场景都要额外兼容两种形态，也容易把"归一化差异"误读成"实现差异"。
- 选项：**(A) 全量重录一次**（33 场景 × 3 次 ≈ 96 次运行），彻底统一；**(B) 保持现状**，在差分脚本里兼容两种形态。
- **推荐 A**：一次性成本换掉长期的双形态分支，且重录本身还能顺带复验 mock 修复后的稳定性。

### Q11 的执行情况

- **裁定 A：全量重录**，已在批次 B5 执行——33 个场景各连跑 3 次，全部 `sane=true`；对比报告见 `notes/rerecord-diff.md`（32 个真实场景 × 2 层 = 64 项，**一致 62 / 不同 2**，唯一差异是 `S10` 的**残缺失基线**——旧 golden 是在有收尾缺陷的 runner 下录的）。
- 归一化形态现已统一（`responseId` 整键抹除），差分脚本不需要保留两种形态的分支。

## Q12（已裁定并关闭）：skills 与 project_context 的先后顺序

- 现象：`skills` 段只在 S24 出现、`project_context` 段只在 S23/S24 出现，**两者从未在同一场景同时出现**，因此它们在系统提示里的相对顺序无法从现有 golden 判定。我当前把 `skills` 排在 `project_context` 之前是**猜测**（已在 `system_prompt.rs` 的常量文档里标明）。
- 选项：**(A) 新录一个场景**（同时带 skill 与项目 AGENTS.md，约 3 次运行）把顺序钉死；**(B) 不录**，保留"未验证"标注。
- **裁定：选 A，已执行**。新录了 `golden/S23b`（全局 skill + cwd 的 AGENTS.md 并存），连跑 3 次稳定。
- **结果推翻了我此前的猜测**：并存时顺序是 **`project_context` 在前、`skills` 在后**，而我原先把 `skills` 排在前面。`SECTION_ORDER` 已修正，测试与 `spec/g-system-prompt.md` 已同步。

## Q13（假设已由 Q14 解释，待 S09a 重跑验证）：S09a 的 abort 为何总落在请求之前

- 症状：`S09a`（prompt → 等到流事件 → abort）跑出来 `gateHits=0`、`requests.jsonl` 为 0 行、assistant 为 `aborted` 且文本为空——**mock 从未收到请求**，所以它仍然没有覆盖"取消已发出的请求"。
- 已排除的做法：
  1. abort 原先写在场景的 `commands` 里——但 `commands` 是 runner **在脚本运行前一次性预发**的（`run.mjs:351`），因此必然抢在请求之前。**已改为只在脚本里发**，问题依旧。
  2. `gateAfterChunks` 此前因 serde 静默忽略而从未生效——**已在 D0 修复**并加了 `chunkByEvent`，S07/S08/S09 的注入断言都能通过，说明闸门本身可用。
- **尚未解释的矛盾**：`S09a` 的 `events.jsonl` 里**完全没有 `message_update`**（逐行统计确认），但脚本却越过了 `waitEvent("\"type\":\"message_update\"")` 这一行继续执行（事件顺序显示 abort 的 `response` 出现在 `agent_settled` **之后**）。
  - 若 `waitEvent` 超时，`queue.take` 会 reject 并让整次运行失败（脚本循环里没有 try/catch 吞异常），但它没有失败。
  - 因此"为什么它没超时"这一点无法自洽解释——**不排除我对 runner 事件流与脚本执行顺序的理解有误**。
- 处置：**假设**是 Q14（`waitEvent` 从不检查匹配条件，于是脚本在第一个事件行上就等到了、abort 随即发出）。Q14 的根因已修复；该假设待 S09a 用修好的 runner 重跑验证。S09a 目前仍标"不作基线"，现场（`golden/S09a`、`golden/_runs/S09a`）保留。

## Q14（已关闭）：`waitEvent` 等不到事件时 runner 不失败

- **根因**：`EventQueue.push` 把**任何**新到达的行交给最老的等待者，**完全不检查 `match`**：
  ```js
  const waiter = this.waiters.shift();
  if (waiter) waiter.resolve(line);   // ← 与 match 无关
  ```
  于是 `waitEvent("永不出现的事件")` 会被下一条任意事件行（`agent_start`、`turn_start`…）唤醒，等待原语形同虚设。这同时解释了 S09a 的矛盾。
- **修复**：`push` 改为 `findIndex(w => line.includes(w.match))`，只把匹配的行交给对应等待者，不匹配的行照常缓冲；新增 `failAll(reason)`，在子进程退出与输出流关闭时让所有等待者**失败**（而不是当作满足）；`#matched` 的等待者带上 `match` 与 `reject`。队列抽到 `tools/difftest/primitives.mjs`，供自测导入。
- **验证**：负向对照（`waitEvent("永不出现")`，`DIFTEST_TIMEOUT_MS=5000`）现在**退出码 1**，错误写明 `等待事件 "type":"this_event_never_happens" 超时`。回归：S01、S09b、S10、S24b 仍 `sane=true`。
- **自测**：`tools/difftest/test/queue.test.mjs`（`node --test`，无新依赖）覆盖"匹配成功 / 超时失败 / 流关闭失败 / 子进程退出失败 / 缓冲命中 / 多等待者互不串线 / failAll 后新等待不受影响"，**6/7 通过**；第 1 条的失败表现为 unhandledRejection（pending 在第二次 push 后仍超时），**疑在测试自身的 promise 记账，而非原语**——未确认，如实留下。
- **仍未做**（P3.-2 的后续要求）：用修好的 runner 重录所有使用等待原语的场景（3 次）并写 `notes/runner-fix-diff.md`；逐条复核 spec/decisions 里据此得出的结论（S09b 通过、S10/S11 稳定、S16、S24 系列）——**这些结论现在一律视为"待重新确认"**。

## 原始记录（保留）：Q14 的发现过程

- **负向对照**（按 RUNBOOK-4 的 P3.-1 第 1 步做的）：写一个场景，让 `waitEvent` 去等 `"type":"this_event_never_happens"`（永不出现），命令 `DIFTEST_TIMEOUT_MS=8000`。预期：runner 因超时失败、非零退出。
- **实测结果**：runner 报 `第 1/1 次完成`、`sane=true`、`stableLayers={"A":true,"B":true,"C":true}`、**退出码 0**。**没有失败。**
- 这正是 P3.-1 预警的情形（"如果它没有失败，这本身就是缺陷，先修"），**也是 S09a 那个矛盾的答案**：脚本里的 `waitEvent` 等不到事件时并没有让运行失败，而是继续往下走，于是 abort 落在 `agent_settled` 之后、mock 从未收到请求。
- 代码路径初查（未定位到根因）：
  - `withTimeout` 的实现在 `run.mjs:100-104`，超时时 `reject`，看起来正常；
  - 脚本整体包在 `await withTimeout((async () => { for (const step of script) {…} })(), \`场景脚本 ${id}\`)` 里，**没有 try/catch**；
  - 因此按代码读，超时应当让 `runOnce` 抛错。**实际却正常返回**——矛盾点在此，尚未查明（可能与我先前改动的收尾/回收逻辑有关，也可能是脚本循环之外有捕获）。
- **影响面**：所有依赖 `waitEvent`/`waitState` 外部化时序的场景（S09、S09a、S10、S11、S16、S24b 等）都建立在"等不到就失败"这个前提上。**前提不成立意味着这些场景的时序断言可能是空的**——包括此前几轮据此得出的结论。
- 处置：**不猜测**。这条缺陷优先于 P3.-1 的后续步骤与 P3.0，必须先在 runner 里修掉（或在脚本里显式断言"注入确实发生"作为补偿），再继续。

## Q15（已关闭，仅剩 NFKC 这一项已知限制）：edit 的模糊匹配——规则已抽取实现，NFKC 仍缺

- 现象：`fuzzyFindText`、`normalizeForFuzzyMatch`、diff 生成这些函数**不在 npm 包的公开导出里**（只导出 `createEditToolDefinition`，其 `EditOperations` 类型在 bundle 中被抹掉）。因此无法像 truncate 那样建 oracle。
- 现有覆盖：`t3_21_edit_golden` 的 S04/S04b/S04c 全是**精确匹配**；模糊匹配（`oldText` 与文件内容有空白/缩进差异时如何匹配、命中后如何保留未变更部分）**既无 oracle 也无 golden**。
- 处置：需要新增一个"模糊匹配"的 golden 场景（让 TS 对一份缩进不同的文件做 edit，抓取结果与请求体），或读 `edit-diff.ts` 源码把归一化规则抽成 spec 条目。两者都未做。

### Q15 的关闭情况

- **已做**：从 `edit-diff.ts:11-57、207-251` 抽出并实现了换行归一化、`normalize_for_fuzzy_match` 的显式替换规则、`fuzzy_find_text`（精确优先 + 模糊兜底）、`count_occurrences`（**在归一化空间里数**），8 例测试。
- **仍未覆盖**：**NFKC**（全角/兼容字符的模糊匹配）。需要完整的 Unicode 兼容映射表，手写子集必然静默产生与 TS 不一致的结果；要补就得引入 `unicode-normalization` 依赖，按 AGENTS 需先确认。当前有专门用例把"未实现"钉住。
- 另一个缺口已定位到具体位置：`edit-diff.ts:137-165` 的 `applyReplacementsPreservingUnchanged`——按行分组后**只替换命中区间**，未改动部分保留原始文本（含原始行尾空白）。当前实现是"整段以归一化内容为基准替换"，**会抹掉未改动行的尾随空白**，与 TS 不一致。补齐需要约一百行（行切分、行区间、分组合并），未做。

### Q15 的关闭依据（edit oracle，40 组）

`tools/oracle/edit-oracle.mjs` 调 TS 包真实的 `createEditToolDefinition`，40 组专为边界构造的
用例（重复/部分行覆盖/CRLF/NFKC/空文件/多编辑组合…），落成 `fixtures/edit/cases.json`；
`crates/pi-core/tests/t3_23_edit_oracle.rs` 用同一批输入驱动 Rust `EditTool` 逐字节比对改后文件内容。

- **38/40 逐字节一致**（含当初担心的"整行粒度 vs 字节偏移"全部场景）。
- **抓到 1 个真缺陷**：`oldText` 用 LF、文件是 CRLF 时走模糊路径，`\r` 被当行尾空白修剪掉，**整份文件的 CRLF 被降级成 LF**（TS 保留）。根因是 `detect_line_ending`/`normalize_to_lf`/`restore_line_endings` 写了但没接进 `apply_edits`。已按 TS 流程接线。
- **抓到 1 个 panic**：模糊路径用 `end - 1` 找"命中最后一字节所在行"，多字节字符时落在字符中间（`中文\n第二行` 的 end=13 → 12 在「二」里），`line_of_offset` 的切片直接 panic。已改为取最后一个**字符起点**，并给 `line_of_offset` 加边界回退（算错偏移只偏一行，不打崩工具）。
- **2/40 是已知差异（NFKC）**：TS 第一步是 `text.normalize("NFKC")`，全角 `Ａ` 折成半角 `A`；我不实现。测试里用 `known_differences()` 白名单**双向断言**：表内必须真的不一致（将来实现了 NFKC 会失败并提示删表项），表外必须逐个一致。

### Q8 的关闭

用户裁定：**不引入 `preserve_order`**。`models.json` 字符级保序做不到（serde_json 默认按 key 排序、不保留原始顺序），
接受"语义等价"。配套规则已定：任何复制/重写会话文件的场景必须**逐行原样拷贝**，不得经解析器往返。

### NFKC 的处置

用户裁定：**暂不实现**，转为**已知限制**，不再作为待决项挂在 Q15 下。
已落地的做法：`t3_23_edit_oracle` 的 `known_differences()` 白名单双向断言——表内两条（全角/半角折合）
必须**确实不一致**（将来若实现 NFKC，测试会失败并提示删表项），表外 38 条必须逐字节一致。

## Q18（已确认并落地）：`message_update.usage` 的差异定为"chunk 边界 artifact"——比较器只在该两处忽略 usage

机制已查清（见 `decisions.md`）：TS 与我的实现**同构**（整块解析 + 发事件时浅拷贝共享的 partial 对象），
差异只来自**网络 chunk 边界**——golden 录制时第 1 行与其余行落在不同 chunk，我的客户端收在同一个 chunk。
因此它**不是实现语义差异**，也不是改代码能对齐的。建议按"环境 artifact"处理，
B 层那 13 个场景的 usage 差异按 PLAN 第 3 节的折叠规则视为可接受；**请你确认**。

## Q16（已关闭）：`auto_retry_start.errorMessage` 与 SDK 文案对齐

已知 golden 记的是 OpenAI SDK 的原文（`"429 status code (no body)"`），而 pi-rs 用自己的措辞
（`"provider 请求失败: HTTP 429 Too Many Requests: {}"`）。**要对齐需要 SDK 源码**（不在 `ref/` 内）
或**更多样本**（不同 status/body 组合）才能反推规则。当前作为**已知差异**记录，不猜。
差分覆盖不到它（B 层只比类型与顺序）。

### Q16 的关闭依据

- **规则出处（读码）**：`pi-ts-0.87.1/node_modules/@earendil-works/pi-coding-agent/dist/bundle/chunks/chunk-NUHFSC37.js`
  的 `APIError.generate(status,errorResponse,message,headers)` 与 `APIError.makeMessage(status,error,message)`。
  关键一条：`generate` 取的是 **`errorResponse?.error`**，不是整个响应体。
- **实现**：`pi_ai::provider::openai_error_message(status, body)` 照抄三条分支；`ProviderError::Http` 增加
  `message` 字段（Display 用它），并在 `to` 构造时就定型。
- **5 例单测**：`{}` → `429 status code (no body)`、空串 → 同、`{"error":{"message":"boom"}}` → `400 boom`、
  非字符串 message → `400 {"code":"x"}`、只有 type → `403 {"type":"forbidden"}`、空字符串 message → `no body`。
- **端到端**：S07 的 `auto_retry_start.errorMessage` 现在是 `"429 status code (no body)"`，与 golden **逐字一致**
  （此前是 `"provider 请求失败: HTTP 429 Too Many Requests: {}"`）。顺带修掉 `LoopError` 的 `"provider 请求失败: "`
  前缀——它不该出现在这个字段里。

## Q17（已关闭）：edit 的 `details`（diff/patch）——**手写移植 jsdiff**（不新增依赖，Q17 定案）

**事实（已查证）**：
- `ref/.../core/tools/edit-diff.ts:376-380` 的 `generateDiffString` 用 `Diff.diffLines`
  （npm `diff` 包，Myers 行差分），再自行为每行加行号与上下文（默认 `contextLines = 4`）；
- 同文件 `:365-370` 的 `generateUnifiedPatch` 用 `Diff.createTwoFilesPatch(…, {context: 4, headerOptions: FILE_HEADERS_ONLY})`；
- 两者都 `import * as Diff from "diff"`（同文件 `:5`）。

**影响**：`tool_execution_end.result.details` 里带 `{diff, patch, firstChangedLine}`，
是 B 层比对的一类差异（S04、S26 等）。我的 `tool_result_json` 已支持 `details`，但 **edit 工具产出不了**。

**需要你决定**：
- (a) 批准新增一个 Rust 差分 crate（如 `similar`），并接受其输出与 npm `diff` **可能需要逐字符调校**；
- (b) 我手写 Myers 行差分 + 统一 diff 格式化（无新依赖，但要与 npm `diff` 的输出逐字符对齐，
  工作量与风险都不小）；
- (c) 暂不做，把 `details` 记为已知缺口（B 层这几处差异保留）。

**我的建议**：选 (a) 并同时保留 golden 对照——因为"无依赖"在这里要换来自写差分 + 对齐 `diff` 包的边角行为
（如空行处理、`\ No newline at end of file`、context 合并规则），风险高于加一个成熟 crate。

## Q19（已证实）：S09 运行失败的机制

**现象**（已证实）：S09（rpc：`commands=[prompt, abort]` + 脚本等 `"type":"text_start"`）在 pi-rs 上
**一个事件都不发**，脚本等待超时 → 运行失败；golden 早已因匹配器缺陷作废（`golden/_invalid/S09/`）。
**TS 的行为**：同样立刻 abort，但 TS 仍发出 `text_start`（其 golden 里有），所以该场景在 TS 侧成立。

**已证实（P4.9+）**：abort 在请求起步前到达，因此**没有发出任何 HTTP 请求**。证据链：
① runner 现在在失败时也保留产物（含 `mock-requests.jsonl`）；② 该文件在 S09 的失败产物里**不存在**；
③ mock provider 的请求日志是 `OpenOptions::create(true)` **在记录请求时才创建**（`tools/mock-provider/src/main.rs:269-270`），
所以"文件不存在"等价于"**没有任何请求到达 mock**"。因此 pi-rs 侧一个 chunk 都没有，`text_start` 自然等不到。
**影响**：S09 的 golden 早已作废，本场景暂时无法作为基线；要恢复需先定"abort 落在在飞请求上"的事件契约。
**影响**：S09 目前无法作为基线；若 P5 要恢复它，需先定"abort 落在在飞请求上"的事件契约。

## Q20（已关闭）：`i-cli.md` 里两处行内断言与实际不符

写规格回写时核对到两处**行内 `**待验证**`**（不是 `【待验证】`，所以不在 23 条计数里）的说法与实际不一致：

1. ~~`i-cli.md:29` 说子命令「由 `main.ts` 在参数解析前分派」~~——**已更正（P4.9+）**：main **确实**在解析前分派
   （`runAuthCommand` 的调用点在 `main.ts:575`，主 `parseArgs` 在 `:607`），只是**具体实现**散在各模块：
   `auth`→`cli/auth-command.ts`、`config`→`cli/config-selector.ts`、包管理类→`package-manager-cli.ts`；
   而 `cli/experimental/command.ts:80` 的 `subcommands` 映射是**扩展命令**那套，与这七个子命令无关。
   结论：原条目说法基本成立，我此前「不准确」的判断是错的。
2. ~~`i-cli.md:47` 说「参数错误类退出……共 **8 处**」~~——**已逐处归类并更正为 16 处**：`main.ts` 里
   `process.exit(1)` 共 16 处，类别含参数冲突（`--fork`/`--session-id`）、会话解析与「找不到会话」、
   会话 id 已存在、`--name` 空值、RPC 模式不支持 `fileArgs`、会话 cwd 缺失、启动基准仅限交互模式、
   无可用模型等；原文的「8 处」无法复现，spec 行已按实测改写。

## Q21（P4.9+ 提出，待定位）"工具抛错 → 工具结果"的转换点

`e-tools.md` 原来的待验证条目把"错误 → 工具结果 的转换"归到 `wrapToolDefinition`，但实测
`core/tools/tool-definition-wrapper.ts`（47 行）只做 `ToolDefinition` ↔ `AgentTool` 的两个方向**投影**
（反向那个注释还明确说**不含** prompt 元数据与 renderer），**没有** catch/错误转换。
**待定位**：抛错转成工具结果（及 renderer 绑定）究竟在哪——候选是 agent 侧的
`prepareToolCall`/`finalizeExecutedToolCall`（`packages/agent/src/agent-loop.ts:546/601` 一带）。
**影响**：只影响 spec 的归属描述，不影响本项目的实现（我们的 `Tool::invoke` 错误路径已按 golden 对齐）。

## Q22（P4-5o 发现，待修）比较器会把"两侧都失败/都缺"判成 `same`（**假通过**）

**现象**：`scenarios/S32.json` 第一版漏了 `prompt`（rpc 场景必须有 `prompt`，否则 runner 等不到 `agent_settled`），
于是 pi-rs 与 TS **两侧都运行失败**：产出目录里只有 `FAILED.md`、`events.jsonl`/`session.jsonl`/`requests.jsonl` 全是 **0 字节**。
`node tools/difftest/compare.mjs S32 --out tmp/s32` 却报了 **`A=same B=same C=same fs=same exit=same`** ✗。

**为什么危险**：这正是本项目反复强调的"**测试通过不等于被测路径真的走到了**"——
两侧都空时"A 层逐行比较"会退化成"空 vs 空"，把一个**运行失败**伪装成**全同**。

**要改**：`tools/difftest/compare.mjs` 在比较前先做**存在性/有效性闸门**：
任一侧缺文件、或文件为 0 字节、或产出目录里存在 `FAILED.md` → 该场景一律判 `different`/`error`（并打印原因），
**不得**判 `same`。加**负向对照**测试（两侧都空 → 必须不是 `same`）。

**已规避**：S32 已补 `prompt` 并重录基线，现在是真通过（两侧 `session.jsonl` 各 9 行）。
**注意**：这类闸门属于"放宽判据的反面"（收紧），不违反"不得放宽"的规则，但**必须写进 decisions**。


## P4-13 新发现（待查/待定）

- **S10 顺序竞态（P4-14 步骤 2 诊断为「未诊断」：26 次复跑未复现）**：全量跑里出现一次 B 层不同——第 12 行 golden 是
  `{"args":{"command":"while [ ! -f flag.txt ]; do sleep 0.05; done; echo done"},"partialResult":{"content":[]},"toolCallId":"call_wait","toolName":"bash","type":"tool_execution_update"}`，
  pi-rs 是 `{"followUp":[],"steering":["改用中文回答"],"type":"queue_update"}`；
  同一二进制立刻复跑 3 次全部五层 `same`，golden 三次录制逐行相同。
  → 怀疑是"长跑 bash 工具 + steering"下 `tool_execution_update` 与 `queue_update` 的**顺序竞态**（两侧都可能抖）。
  **待查**：是否需要像 S16 那样把场景的等待点改稳，或在比较器之外另加顺序规则（未决，本批不动）。
- **S23b / S24 在本批全量脚本里的一次"运行失败"**：脚本把它判为 run 失败，单独重跑两次均正常（`run.sh` 退出码 0，
  `sane=false` 是场景本身的预期形态）。原因未定位（可能是当时的资源/时序），**待查**。
- **S09 的失败形态可变**：P4-11 记的是"从未发出 HTTP 请求"（产出目录里没有 `mock-requests.jsonl`）；
  P4-13 这次失败时目录里**有** `mock-requests.jsonl`（即请求发出过），失败文案是
  `等待事件 "type":"text_start" 超时`。→ S09 的"abort 落在请求起步前"这一结论**只在当时的运行里成立**，
  不能当作不变结论；需要单开决策卡时重新取证。


## P4-14 步骤 2 的更正与诊断

- **S10 改判「未诊断」**：顺序 20 次 + CPU 负载下 6 次，B 层全部 `same`（原始输出见
  `notes/p414-step2-s10.txt`、`notes/p414-step2-s10-load.txt`）。唯一一次差异的产物里，
  序列是 `…tool_execution_start, queue_update, response, tool_execution_update…`（golden 是
  `…tool_execution_start, tool_execution_update, queue_update…`），即两个事件的**先后**不同；
  生产者为 `rpc_session.rs:221`（steering 入队 → `queue_update`）与 `run.rs:772-779`（工具进度 →
  `tool_execution_update`）两条异步路径。**不得**判"偶发"，标"未诊断"。
- **S23b/S24 更正**：10/10 `run.sh exit=2`（`产出未成形: session 为空`），golden 是 `sane=true/exitCode=0`
  → 属 P6 未实现功能导致的**确定性**差异。P4-13 记的"重跑退出码 0"是**管道掩盖 `$?`** 造成的错误记录。
- **S09 / Q19 结论作废**：`golden/_runs/S09/run-1` 的 mtime 是 2026-10-06，而 runner 从 `4e8418e`（2026-10-09）
  才开始写失败时的 `mock-requests.jsonl` → "产物里没有该文件"**推不出**"没发过请求"。
  现状（6/6 确定性）：发出 1 次请求后等待 `text_start` 超时；**TS 侧同样失败**，且 `golden/S09` 不存在
  → S09 是**场景自身**的问题（两侧都过不了等待点），不能当验收场景用。根因未诊断。


## P5-1e E2：工具参数键序 —— **不批准 ①（全局 preserve_order）**，③ 也不可行 ⇒ 走 ②

用户决策（P5-1e）：不批准全局 `preserve_order`；先诊断，③（在 compaction 内用自定义 `Deserialize`
保留顺序）仅当核心类型改动面不大时才做。

**诊断结论（`notes/p5-1e-e2-trace.md` 有完整表）**：参数**原文在 provider 解码那一步（`crates/pi-ai/src/decoder.rs:258`，
SSE 文本经 serde 解析）就已丢失**，而 `serialize_conversation` 只拿得到 `&[Message]` ⇒ ③ **不可行**
（visitor 看不到原文）。要修必须改核心类型（`crates/pi-ai/src/types.rs:94` 的
`Content::ToolCall.arguments: serde_json::Map<String, Value>` 换成保序/原文类型 + SSE 解析路径），
生产代码里触及 `arguments` 的**站点 17 处**，并波及 `pi-ai`（types/decoder/context/message）、
`pi-core`（run/tools）、`pi-cli`。**按指令不实施**，退到 ②。

**②的处置**：登记为已知限制（只影响**多参数**工具调用在摘要文本里的键序；单参数不受影响，
已用专门样本逐字节对拍），并加 `#[ignore]` 用例
`serialize_multi_key_args_diverge_from_the_oracle_by_key_order` 钉住分歧（解除 ignore 即暴露）。
**数值/字符串格式的差异已单独修好**（`js_stringify`，18 条单值样本与 JS 逐字节一致）——它不需要保序。

## （历史）P5-1c：工具参数键序（`preserve_order`）—— 已按上面处置

`serialize_conversation` 里 ref 用 `Object.entries(block.arguments)`（**插入序**）拼 `name(k=v, …)`；
pi-rs 的 `ToolCall.arguments` 是 `serde_json::Map`，未开 `preserve_order`（该 feature 此前**未获批准**）时按
**键名字典序**迭代 ⇒ **多参数**工具调用在摘要文本里的键序可能与 TS 不同（单参数不受影响）。
影响面：只影响 P5 摘要请求的**序列化文本**（`serializeConversation`）；S19–S22 的摘要样本用的是
`read` 这类单参数调用（oracle 已单独录一条单参数样本用于逐字节对拍）。
处置选项：① 申请开 `serde_json` 的 `preserve_order` feature（会改变全仓库的 `Value` 键序语义，需重新核对
所有字节级比较）；② 保持现状并把"多参数键序"记为已知限制（P5-2 若因此对不上再上报）。
**本步不擅自决定**，等人工确认。

## P6-3 / S0(b)：真实 skill 的 frontmatter 兼容性缺口——两个方案（**待人工确认，本批不实现**）

**缺口**（S0(a) 实测，一致率 53.8%）：6/13 个真实 `SKILL.md` 与 ref 解析结果不一致，机制两类——① 折叠块标量（`description: >` / `>-`，4 个文件）；② 双引号串内 `\"` 未反转义（2 个文件）。

**方案 A：扩展 pi-rs 的最小 YAML 子集**（仍不新增依赖）
- 改动量（估）：`crates/pi-core/src/skills.rs` 的 `parse_frontmatter_minimal` 约 +60～100 行——① 识别块标量（`key: >`、`key: >-`、`key: |`、`key: |-` 及其缩进续行；`>` 折叠为空格、`|` 保留换行，`-` 去尾换行）；② 双引号串的 `\"`/`\\`/`\n` 反转义与单引号串的 `''`；③ 续行/缩进的边界处理。测试 +4～6 条（每类一条正例与一条反例）。
- 风险：中。仍是**手写 YAML 子集**，真实世界还会遇到更多形态（`>` 与空行交互、多行 plain 标量、锚点/别名、注释在值内、`: ` 出现在值里等）；每遇到一种就要再补一次，且「与 `yaml.parse` 完全一致」无法用子集保证。
- 优点：不违反"不新增依赖"的硬约束；对当前 13 个文件可做到 13/13 一致（含 `>`、`>-`、`\"` 三类）。
- 缺点：长期仍有隐性不一致；需要把"子集范围"写进文档并在遇到不支持形态时**显式诊断**（而不是静默给错值）。

**方案 B：新增 YAML 依赖**（如 `serde_yaml`/`saphyr`/`yaml-rust2`）
- 改动量（估）：`Cargo.toml` +1 依赖（及其传递依赖）、`skills.rs` 删掉手写解析（-40 行左右）、测试沿用；另外要满足 AGENTS.md 的"允许依赖清单"与 musl 静态链接检查（`cargo tree -i openssl-sys` 仍须不报）。
- 风险：中高。① 违反 AGENTS.md「需要新增上述之外的依赖：先停下，在 notes/questions.md 写明理由，等人工确认」⇒ 必须你放行；② 依赖体积与供应链面扩大；③ 与 ref 的 `yaml` 包版本行为差异仍需回归（例如 `yaml.parse` 对重复键/类型强转的处理）；④ 目标平台是 musl 静态链接，需确认该依赖的纯 Rust 属性。
- 优点：与 ref 的解析语义**一次性对齐**（block scalar、引号转义、多行 plain、注释、锚点等），后续不再反复补洞。

**我的建议**：先做**方案 A 的受限版**（只覆盖实测到的 ① 折叠块标量 ② 双引号转义，并把"不支持的形态 ⇒ 显式 warning 诊断 + 不加载"写死），因为本批/下批的 golden 都不依赖复杂 YAML；同时把方案 B 作为"若再遇到第 3 类形态"的备选，等你决定是否加依赖。

## P7-4 / W1：T07（Escape 中断运行中的请求）缺少 runner 原语
`tools/mock-provider` 的场景脚本原语只有 `responses`（按顺序回放）、`gateAfterChunks`、`disconnectAfterChunks`、`chunkByEvent`（见 `tools/mock-provider/src/main.rs` 的 `Step` 结构），**没有"保持流不结束并挂住"的语义** ⇒ 无法构造"运行中请求"以便按 Escape 中断。按批次要求：**跳过 T07**，不擅自改 mock 判据；如需支持，请裁定新增原语（例如 `holdOpenAfterChunks: N` 或 `neverFinish: true`）。

## P7-6j/W3 待确认（2026-xx，验证前不得实施）
1. **W3 harness 是否入库**：建议放 `tools/difftest/tui-oracle/`，只提交 `run.mjs`/`run2.mjs` + 用例 JSON，**不复制上游源码**，运行时从只读 `ref/pi-0.87.1/packages/tui` 导入；不新增 npm 依赖。是否允许？
2. **渲染器开关形态**：`--tui-renderer=r1|r2`（`--help` 可见，CLI 旗标）vs 沿用既有 `PI_TUI_RENDERER` 环境变量。批次要求"不接受隐藏环境变量"⇒ 若沿用环境变量是否算"隐藏"？
3. **fullscreen 处置**：本阶段继续维持"拒绝 `--tui-mode`"（S23b/S24 现状），还是在本阶段解除并实现 fullscreen？

## P7-6p(续3)/W2 待确认：r3 运行时每帧 lines 的获取方式

**背景**：W2.1 的重建法校验**不通过**——用 r3 raw 的末块字节前缀重建得到 **32 行**（探针 `t05_state_tail_probe` 为 **31 行**），多出的是**尾部空行**；W2.2 的逐块字节比对也因此**全部不一致**（14/14 块），且差异形态（TS 多写空行/多 `\e[17A` 光标移动）表明**重建出的行数组不等于 r3 当时真正喂入的行数组**（内容对得上，但差分操作对不上）。

**需求**：要完成 W2.2 的逐块字节比对，需要 r3 **运行时**每帧实际喂给 `diff_render` 的行数组（含 marker 与宽高）。

**备选方案**：
1. **编译期 cargo feature 门控的 dump**（如 `--features tui-dump-lines`）：默认构建**不含**、不改变生产行为；开启时 r3 路径把每帧 lines 写入 `/tmp`。**需批准**（会动 Cargo.toml 与 pi-cli 的 r3 分支）。
2. **env 门控 dump**（`PI_RS_TUI_DUMP_LINES=1`）：改动更小，但属"新增环境变量开关"，需批准是否违反"不新增 CLI 旗标/开关"的本意（它不是 CLI 旗标，但同样是新开关）。
3. **放弃逐块字节比对**：以 W1.3 的块级滚动表 + W2.1 的"+1 行=末块多一次换行"为最终结论，B 组按**未定位**收尾。

**当前状态**：W2.2 及其后续（W2.3–W2.5）**已按批次规则停止**（重建法无效），未做 W3–W7 中依赖 W2 的部分。

## P7-6p(续7)/W3 待确认（#15 面板宽度）——**已裁定（P7-6r0：采纳方案 C，夹具入库，分档仅过渡须登记）**
1. 是否接受"**仅支持 cols=80/100 两档 + 其它宽度登记为已知限制 #15**"作为切默认前提？
2. 若要全宽度：是否批准按**方案 C**（临时目录用真实 TS 组件在多宽度下渲染，产出夹具）？需先评估 `SettingsSelectorComponent`/`SettingsList` 能否脱离完整应用实例化（不能则按方案 B 复刻逻辑）。
3. 是否批准在**临时目录**（`unshare -n`、不入库）用 TS 侧重新录制 **cols=80** 的面板 golden，供方案 A 转录？

## P7-6q/W4（待裁决）——**已裁定（P7-6r0：宽度夹具入库为 fixtures-width.json；/hotkeys 走方案 C）**
- **宽度夹具是否入库**：W4 已证明真实 TS `Markdown` 组件可在 identity 主题下、于临时目录渲染出 cols=60/80/100/120 的行数组（`maxWidth` 被 clamp 到 width）。是否把该产物入库为正式宽度夹具（`tools/difftest/tui-oracle/fixtures-width.json`），还是仅留临时目录？**待人工确认**。
- **`/hotkeys` 表格文本来源**：有效性检查（cols=100 复现 golden `12..101` 区间）需先从 ref 取出 hotkeys 表格原文；该文本在 `interactive-mode` 侧的生成函数位置**待验证**。

## P7-6r1/W4（待确认）：cols=80 录制方案——**已裁定（P7-6r2：批准新增 T15c80 场景并录 golden，不改 runner）**
- **更正**：`cols` 已是**场景字段**（`scenarios-tui/*.json` 的 `tui.cols`，`run-tui.mjs:44` → `pty-helper.py:73`），**无需新增 runner 参数**；P7-6r0 的"W4 停止"系误判。
- 待确认：是否**新增 `scenarios-tui/T15c80.json`（仅 `tui.cols=80`）并录制新 golden**，用于 `/hotkeys` 面板的 cols=80 PTY 级交叉校验（与 `fixtures-width.json` 的 80 档比对）。不批准则维持"仅夹具驱动、不经 PTY"。
- 另待确认（W3 后续）：是否投入构造 `SettingsSelectorComponent` 的 38 字段桩 + 模型运行时快照，以产出 `/settings`、`/model` 的宽度夹具。

## P7-6r1 待决策三项（P7-6r2 已裁定）
- ① T15c80 场景与 golden：**已批准**（仅 `tui.cols=80`，不改 runner）。
- ② `SettingsSelectorComponent` 38 字段桩：**不批准**；`/settings`、`/model` 窄宽度登记"未覆盖"。
- ③ #15 Rust 动态渲染移植：**已批准**，本批实施 `/hotkeys` 部分。

## P7-6s 已裁定（对应 P7-6r2 收尾的三项）
- ① `run-tui.mjs` 的 `--impl` 失效：**已批准修复**（仅让 `--impl` 生效 + 冲突/未知参数报错退出）。
- ② `/settings`、`/model` 窄宽度：**继续不覆盖**，登记为切默认阻塞项。
- ③ 进入终端协商取证：**已批准**（本批 crates/ 零改动）。

## P7-6t 已裁定（对应 P7-6s 收尾的三项）
- ① r3 验收口径：**统一为 TS golden 严格逐帧 same + 未折叠全文 same，各 3 次稳定**。
- ② P7-6p(续7) 的"r3 全 same"：**正式降级为待重验**（已写入 P5-9 与 limits）。
- ③ 协商状态机移植（P7-6u）：**顺延**，本批只定位。

## P7-6u 已裁定（对应 P7-6t 收尾两项）
- ① 修复顺序：**先定位 #16，再重验 #13**。
- ② 终端协商移植：**顺延为 P7-6v**；本批只定位 #16。

## P7-6v 已裁定（对应 P7-6u 收尾两项）
- ① 正式全量复验：**批准为本批首要任务**（18 场景 ×3 次 × 两口径，且每次录制先过有效性闸门）。
- ② 终端协商移植：**顺延为 P7-6w**。
- ③ 更正：续7「r3 全 same」与「#13 在 r3 清零」改回**待当批复验**（P7-6u 的撤回只有 7×1 次证据）。

## P7-6w 已裁定（对应 P7-6v 收尾两项）
- ① 作废证据：P7-6p(续7) 与 P7-6q W0 的"来源缺失"证据**不重录**，标"已被 P7-6v 当批复验取代"。
- ② 终端协商状态机：**开始移植**（纯函数、不接线）；r3 表述绑定 HEAD=`1efc782`。

## P7-6x 已裁定（对应 P7-6w 收尾两项）
- ① 接口边界：**先裁定再接线**（本批 W1 出裁定）；OSC 11 oracle 与接线**分批**。
- ② r3 表述绑定 HEAD=`1efc782`；本批 crates 零改动。

## P7-6y 已裁定（对应 P7-6x 收尾两项）
- ① 接口边界：**批准 on_data → on_sequence 重构**（删内部切分与括号粘贴）。
- ② OSC 11 与 kitty 协商**两个独立状态机**、分别实现；**接线顺延 P7-6z**。

## P7-6z 已裁定（对应 P7-6y 收尾两项）
- ① 接线：**顺延 P7-6aa**；接线前先清零 d1/d2 未定位项。
- ② 真机测试约定：**沿用"切默认前不请用户真机测试"**。
- ③ 台账：d1/d2 独立条目，计入未定位（14 条 / 未定位 2 条）。

## P7-6aa 已裁定（对应 P7-6z 收尾两项）
- ① d1/d2 修复：**批准**，仅限 `osc11.rs` 与 `tests/osc11_fixtures.rs`。
- ② 接线：**顺延 P7-6ab**；沿用"切默认前不请用户真机测试"。

## P7-6ab 已裁定
- ① 接线范围：**只进 r3**，R1/R2 不动。
- ② 真机测试：沿用"切默认前不请用户真机测试"。

## P7-6ac 已裁定（对应 P7-6ab 收尾两项）
- ① S2 发现项：先做 **D 步骤（根因测量）**，再 S3/S4，最后 S2b 补接启动序列。
- ② 顺序调整批准：**D → S3 → S4 → S2b → S5 → S6**（RUNBOOK 偏差已记录）。

## P7-6ad 已裁定
- ① 顺序：D → S3 → S4 → S2b → S7 → S5 → S6；S8 只出预案。
- ② D 未定位不阻塞 S3/S4；D.4 由 agent 自做，不作为待决项。

## P7-6ae 已裁定（预先裁定，不等批准）
- ① S8 预案：直接产出。
- ② `/settings`、`/model` 窄宽度：登记为切默认阻塞项，不做桩。
- ③ 前缀兜底：保留。
- ④ 顺序：D → S3 → S4 → S2b → S7 → S5 → S6 → S8；D 未定位不阻塞 S3/S4。


## P7-6ae/S4.2（待确认）：选择性应答注入能力
- 需求：在 pty master 上"看到某查询序列后再按指定时刻/分片写回应答"，用于断言"分片应答/夹在按键间/粘贴内含应答样字节/无应答"四类下 pi-rs 都不泄漏。
- 现状：`pty-helper.py:51-66` 已实现**固定应答**（DA1/kitty/OSC10/OSC11），但**不支持按查询触发+自定义分片**；`--script` 只能按时间键入。
- 选项：①给 `pty-helper.py` 增加 `--inject` 规则（改 runner，需批准）；②新增独立 pty 驱动脚本（需自行复刻 cfg/session/mock 初始化，工作量约 1 个批次）；③维持现状（真实 harness 已覆盖"有应答"路径，四类细分靠 `fixtures-term.json` + `fixtures-osc11.json` 的纯函数层覆盖）。
- 推荐：③ + 后续若需要，再按 ① 申请。

## P7-6ae/S5（待修）：`term_negotiate::is_negotiation_prefix` 过宽
- 现象：r3 下 T14/T16/T17 稳定 different（Esc 迟到 150 ms ⇒ 补全弹层未关 / 面板下移一行）。
- 根因：本实现把裸 `"\x1b"` 判为协商前缀，而 ref `terminal.ts:33-35` 只认 `"\x1b["` 与 `^\x1b\[\?[\d;]*$`。
- 修复（需批准动 `term_negotiate.rs`）：把前缀判定改成 `seq == "\x1b[" || (seq.starts_with("\x1b[?") && 其余仅数字/分号)`；同步 `tests/term_fixtures.rs` 的边界单测（`timeout_at_boundary` 需改用 `"\x1b[?"` 触发）。

## P7-6af 已裁定（预先裁定）
- ① 批准修 `term_negotiate.rs` 与 `input.rs`（须有 ref 行号支撑）。
- ② 批准新增 `dsr996.rs` 并接线 r3。
- ③ 不改 runner/`pty-helper.py`；S4.2 改为新增独立脚本 `pty-inject-check.py`。
- ④ 切默认不在本批，待审 S8。
- ⑤ 更正：helper 会应答查询，"golden 无应答路径"作废。

## P7-6ag 已裁定（预先裁定）
- ① E 端到端注入并入本批。② 窄宽度取选项 A（登记限制、不做桩）。③ 删开关后 `PI_TUI_RENDERER` 被忽略不报错。
- ④ 切默认后无 R1 基线，基准为 TS golden + 切前 r3 录制。⑤ 修改范围按批次命令。

## P7-6ai 已裁定
- ① C2 逐处手工拆分支、禁批量替换；每子提交后 r3 T01/T06/T09 两口径 same。
- ② 顺序 M1 → M3 → E → C2 → C3 → C4 → C5 → C6 → F。
- ③ M1/M3/E 失败不阻塞切默认，但须登记并首段列出。

## P7-6aj 已裁定
- ① C3 按 (a)(b)(c) 三子提交。② 只改一个函数边界、每次 cargo check、失败即 checkout 换更小粒度、禁大段整块替换。
- ③ (c) 先纯重构再整函数删除。④ M1/M3/E 共用 pty-harness.py 骨架，排在 C3 前。⑤ C4 基线 B1=ff371c7。


## P7-6ak 冒烟期间发现（待裁决/待修）：**pi-rs 未加载内置模型目录**
- 现象：用户用**真实** `~/.pi/agent/models.json` 启动 pi-rs 报
  `Provider "openrouter": Provider openrouter: "baseUrl" is required when defining custom models.`
- 根因：其 `models.json` 里 `providers.openrouter`（以及 `command-code`、`opencode-go`）**没有 `baseUrl`/`api`**，
  依赖内置目录补齐；`crates/pi-ai/src/config.rs` 里 `BuiltInCatalog::find`（`:427`）存在，但**全仓没有任何代码
  加载内置目录**（`grep -rn "BuiltInCatalog::load|pi-official-catalog" crates/` 为空）⇒ 自定义模型校验必然失败。
- ref 侧等价校验在 `core/provider-composer.ts:171`，TS 能通过是因为它有内置模型目录（`defaults?.baseUrl`）。
- ⇒ **限制（新增）**：pi-rs 目前要求 `models.json` 里每个 provider 显式给出 `api` 与 `baseUrl`。
- 临时绕过（已用于冒烟环境）：在 `models.json` 显式写 `api` + `baseUrl` 即可。

## P7-6ak 发现：内置模型目录**格式不匹配**（实现前需裁决）
- 用户目录的 `~/.pi/agent/pi-official-catalog.json`（935 KB，42 个 provider）格式是
  `{ "<provider>": { "<modelId>": { "id","name","api","baseUrl","provider",… } } }`（**对象**）。
- `pi_ai::config::BuiltInCatalog::from_json` 期望的是**数组**：
  实测 `BuiltInCatalog::from_json(用户文件)` 报 `models.json 解析失败: amazon-bedrock 不是数组`。
- 生产代码**没有任何地方加载**该文件（`config.resolve()` 传 `None` catalog）⇒ 自定义模型缺 `baseUrl` 即报错。
- **实现方案（需批准）**：
  1. `BuiltInCatalog::from_json` 同时接受 `{provider: [ ... ]}` 与 `{provider: {id: {...}}}` 两种形状（后者按 `id` 取值，保持 `#[serde(deny_unknown_fields)]` 之外的宽松）；
  2. `pi-cli` 在解析模型前尝试加载 `<PI_CODING_AGENT_DIR>/pi-official-catalog.json`（缺失/解析失败 ⇒ 记 `tracing` 警告并退回现状，不 panic）；
  3. 把 `config.resolve(...)` 换成 `resolve_with_catalog(..., Some(&catalog))`，覆盖 `main.rs:180/530/914` 三处；
  4. 新增测试：用户格式目录可解析 + `openrouter/openrouter/free` 能命中 + 缺失文件时退回旧行为。
- 风险：目录 935 KB 每次启动解析（约几十 ms，可接受，或按 mtime 缓存）；不改则**任何依赖内置目录的真实配置都无法启动**（当前状态）。
## P7-7f 待确认
- `T43`/首帧增量渲染：若最小改动无法在保持 T01–T18+T15c80、T26–T46 全 same 的前提下实现，是否接受"只在 `/tree` 场景额外维护视口顶部"的折中？（默认：不接受折中，实施失败即整体回退并登记。）

## Q（P8e-9/H-c）：crossterm 无法收窄到 Windows-only
评审要求"确认 crossterm 仅为 Windows 目标依赖"。实测不可行：Unix 侧同样用它做终端控制
（raw mode / 光标 / 清屏），改成 target 依赖后 `cargo check --target x86_64-unknown-linux-musl`
直接编译失败（3 处未解析导入）。

处置：保留跨平台依赖，并在 `notes/decisions.md` 记录原因；musl 产物仍为 static-pie 且无 openssl。
若评审坚持"Linux 不得链接 crossterm"，需要把 Unix 终端控制改用 `termios`/`ioctl` 自实现（工作量约 1 人日），
请明确排期后再动。

## P9-0-A（2026-11-01）待确认

1. **运行时依赖 node 是否可接受**：宿主形态 A（Node 子进程）要求目标机器有 `node`（本机 v24.21.0）。
   若不可接受，只能走 B/C（内嵌 JS 引擎），这会**新增 cargo 依赖**（`boa`/`quickjs`/`deno_core`），
   需要事先批准；`deno_core`(V8) 在 musl static-pie 上基本不可行（见 `notes/p9-0/acceptance-draft.md`）。
2. **渲染钩子的首版策略**：`renderCall`/`renderResult`/`ctx.ui.custom`/`setFooter` 在子进程方案下
   需要"组件树协议"，工作量大。是否接受**首版降级**（宿主忽略这些钩子、扩展走纯文本回退）？
   本机 9 个扩展里：`custom` 3 处、`setFooter` 2 处、`setWidget` 6 处（其中 2 处传组件工厂）。
3. **热重载是否必须保留扩展内存状态**：子进程重启 = 干净但丢状态（`web-search` 的冷却链、
   `pi-handoff-gate` 的待办）；要保留就得定义"状态导出/导入"帧（官方没有，需要自定）。

## P9-1-B（2026-11-01）待决：jiti 如何分发

`.ts` 扩展要经 jiti 转译（ref `core/extensions/loader.ts:496-501`，`moduleCache:false`），
而 jiti 是 npm 包（2.7.0）。三种分发方式：

| 方案 | 代价 | 推翻条件 |
| --- | --- | --- |
| **随程序打包**（`tools/ext-host/vendor/jiti.cjs`，约 1 MB，随二进制/发行目录分发） | 二进制或安装目录变大；升级 jiti 要重新打包；需定期同步上游 | 若发行体积受限或 `.ts` 扩展不再是主流 ⇒ 改按需下载 |
| 要求用户 `npm i -g jiti` | 零体积；但用户环境可能没 node/npm，且版本漂移会让加载行为不同 | 若用户群都能预装 node 工具链 |
| 运行时探测（先找 `PI_JITI_PATH`，再找 pi 包自带 `node_modules/jiti`，最后回落内置） | 实现最复杂，但兼容"用户已有 pi 安装"的场景 | 若探不到就必须回落到"拒绝加载 .ts" |

**本批（P9-1-B）不实现 `.ts`**：宿主只加载 `.mjs`/`.js` 原生 ESM（Node 直接支持），
jiti 路径在 `pi-ext-host` 里**留接口**（`loader: "native" | "jiti"`，后者返回
`unsupported: jiti_not_bundled`），分发方式待用户定。默认建议：**随程序打包**（自包含、离线可用）。

## F46/U1：段落紧接列表的空行（未修，需单列批次）
- 现象：`段（…）：\n- 甲` 时 TS 不插空行、pi-rs 多一行；`段\n\n- 甲` 时两侧**都**插空行。
- ref 依据：`packages/tui/src/components/markdown.ts:487-497` 的 `case "paragraph"` 依赖
  `nextTokenType !== "list"`。
- **阻塞点**：pulldown-cmark 不提供 `nextTokenType`。实测 `段\n- 甲` 与 `段\n\n- 甲` 的事件序列
  **完全相同**（`Start(Paragraph)/End(Paragraph)/Start(List)`）⇒ 空行信息不在事件流里。
- 已试并撤回的方案（都在 `crates/pi-tui/src/lib.rs`）：
  1. `Start(List)` 无条件不补空行 ⇒ T76（代码块后接列表要空行）、T106（段落+空行+列表要空行）回归；
  2. 记录"上一块是 paragraph" ⇒ 同上，且 T89f/T89h 回归；
  3. 列表首行插入行内哨兵 `\u{2}` ⇒ 该层被判为 loose（项间多空行），T05 回归；
     改行尾哨兵 ⇒ 标记泄漏进正文，**全部** markdown golden different。
- 建议方案：在 `render_markdown_styled_with_width` 之前做一次**块级行扫描**，产出
  "第 N 个列表块是否紧接段落"的位图，再按序消费；需重写块间空行逻辑，估计 150–250 行。
- 推翻条件：若 ref 的 `needsBlockSpacing` 之类实现能在事件层等价恢复，可改回事件流方案。
