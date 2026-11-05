# spec/c-session.md — session 文件格式与上下文构建

来源 `packages/coding-agent/src/core/session-manager.ts`（格式与投影）、
`packages/coding-agent/src/migrations.ts`（启动期一次性迁移，与 session 版本迁移是两回事）。

## 1. 文件与 header

- session 是 JSONL 文件，每行一个 `JSON.stringify(entry)` 后跟 `\n`；`FileEntry = SessionHeader | SessionEntry`，header 就是第一行。〔packages/coding-agent/src/core/session-manager.ts:196-197〕〔packages/coding-agent/src/core/session-manager.ts:1128-1130〕
- `SessionHeader` 字段：`type: "session"`、`version?`（注释写明 v1 session 没有该字段）、`id`、`timestamp`、`cwd`、`parentSession?`。〔packages/coding-agent/src/core/session-manager.ts:43-50〕
- 新建 session 时 header 的 `version` 写 `CURRENT_SESSION_VERSION`，常量值是 `3`。〔packages/coding-agent/src/core/session-manager.ts:41-41〕〔packages/coding-agent/src/core/session-manager.ts:1063-1070〕
- `timestamp` 用 `new Date().toISOString()`；`id` 缺省时由 `createSessionId()` 生成。〔packages/coding-agent/src/core/session-manager.ts:1061-1062〕
- 默认会话目录 = `<agentDir>/sessions/<编码后的 cwd>`；编码规则是先把 cwd `resolvePath`，去掉开头的 `/` 或 `\`，再把其余 `/`、`\`、`:` 全部换成 `-`，最后包在 `--` 里；`getDefaultSessionDir` 会在目录不存在时 `mkdirSync(..., { recursive: true })` 建好。〔packages/coding-agent/src/core/session-manager.ts:585-602〕
- 会话文件名是 `<fileTimestamp>_<sessionId>.jsonl`，其中 `fileTimestamp = timestamp.replace(/[:.]/g, "-")`，即 ISO 时间戳里的 `:` 与 `.` 换成 `-`。〔packages/coding-agent/src/core/session-manager.ts:1078-1081〕
- `--session` 显式给的路径会被原样保留（注释 `preserve explicit path from --session flag`），不走上面的命名规则。〔packages/coding-agent/src/core/session-manager.ts:1053-1053〕

## 2. entry 公共字段与全部 entry 类型

- `SessionEntryBase` 四字段：`type`、`id`、`parentId`（`string | null`）、`timestamp`（ISO 字符串）。〔packages/coding-agent/src/core/session-manager.ts:57-62〕
- `SessionEntry` 是 11 个成员的联合：`SessionMessageEntry`、`ThinkingLevelChangeEntry`、`ModelChangeEntry`、`UsageEntry`、`CompactionEntry`、`BranchSummaryEntry`、`CustomEntry`、`CustomMessageEntry`、`ContextEditEntry`、`LabelEntry`、`SessionInfoEntry`。〔packages/coding-agent/src/core/session-manager.ts:182-194〕
- `SessionMessageEntry`：`type: "message"` 加一个 `message: AgentMessage`。〔packages/coding-agent/src/core/session-manager.ts:64-67〕
- `ThinkingLevelChangeEntry`：`type: "thinking_level_change"` 加 `thinkingLevel: string`。〔packages/coding-agent/src/core/session-manager.ts:69-72〕
- `ModelChangeEntry`：`type: "model_change"` 加 `provider: string` 与 `modelId: string`。〔packages/coding-agent/src/core/session-manager.ts:74-78〕
- `UsageEntry`：`type: "usage"` 加 `kind`（注释举例 `"cache_warm"`）、`provider`、`model`、`usage: Usage`，可选 `note`。〔packages/coding-agent/src/core/session-manager.ts:80-89〕
- `CompactionEntry`：`type: "compaction"` 加 `summary`、`firstKeptEntryId`、`tokensBefore`，可选 `details`、`usage`、`fromHook`（扩展生成时为 true，pi 生成时 undefined/false，向后兼容）、`systemMessage`（该压缩边界处完整的提示与工具状态）。〔packages/coding-agent/src/core/session-manager.ts:91-104〕
- `BranchSummaryEntry`：`type: "branch_summary"` 加 `fromId`、`summary`，可选 `details`（扩展专用，不发给 LLM）、`usage`、`fromHook`。〔packages/coding-agent/src/core/session-manager.ts:106-116〕
- `CustomEntry`：`type: "custom"` 加 `customType` 与可选 `data`；注释明确它**不参与** LLM 上下文（被 `buildSessionContext` 忽略），用途是跨重载持久化扩展状态。〔packages/coding-agent/src/core/session-manager.ts:118-132〕
- `LabelEntry`：`type: "label"` 加 `targetId` 与 `label: string | undefined`（用户定义的书签/标记，允许显式 undefined）。〔packages/coding-agent/src/core/session-manager.ts:134-139〕
- `SessionInfoEntry`：`type: "session_info"` 加可选 `name`（用户定义的显示名）。〔packages/coding-agent/src/core/session-manager.ts:141-145〕
- `CustomMessageEntry`：`type: "custom_message"` 加 `customType`、`content`（`string | (TextContent | ImageContent)[]`）、必带 `display: boolean`、可选 `details`；与 `CustomEntry` 不同，它**参与** LLM 上下文，内容在 `buildSessionContext()` 里被转成 user message。〔packages/coding-agent/src/core/session-manager.ts:147-165〕

## 3. context_edit（0.87.0 新增）

- `ContextEditEntry`：`type: "context_edit"` 加 `targetId` 与 `replacement`；`replacement` 为 `null` 表示把目标从模型上下文里**省略**，为对象 `{ content }` 时只替换其 content 而不改消息元数据。〔packages/coding-agent/src/core/session-manager.ts:174-180〕
- `ContextEditableContent` 的取值域是 `UserMessage["content"] | AssistantMessage["content"] | ToolResultMessage["content"] | CustomMessage["content"]` 四种 content 的并集。〔packages/coding-agent/src/core/session-manager.ts:167-172〕
- 语义标签是 `Append-only change to one earlier entry's contribution to model context`：entry 本身只追加不改写历史，改的是**更早某条 entry 对模型上下文的贡献**。〔packages/coding-agent/src/core/session-manager.ts:174-175〕
- 投影时 `context_edit` 的收集范围限于 `buildContextEntries()` 产出的条目：遍历它们，遇到 `type === "context_edit"` 就 `edits.set(entry.targetId, entry)`。〔packages/coding-agent/src/core/session-manager.ts:551-554〕
- 同 id 的多个 `context_edit` 后者覆盖前者（Map 的 `set` 语义），即"最近一条生效"。〔packages/coding-agent/src/core/session-manager.ts:552-554〕
- `projectContextEntry` 的替换规则：`replacement === null` 直接返回空数组；否则只对 role 为 `user`/`assistant`/`toolResult`/`custom` 的消息换 content，其余 role 原样返回；若 replacement 的 content 是字符串而消息 role 是 `assistant` 或 `toolResult`，会被包成 `[{ type: "text", text }]`。〔packages/coding-agent/src/core/session-manager.ts:519-540〕

## 4. parentId 与叶子规则

- 路径构建 `buildSessionPath`：`leafId === null` 直接返回空数组；给了 `leafId` 就查索引，查不到则回退到 `entries` 的最后一条；再没有则返回空数组。〔packages/coding-agent/src/core/session-manager.ts:390-406〕
- 回溯方式是顺着 `parentId` 向上走直到 `parentId` 为空/undefined 为止，收集后 `path.reverse()` 得到根到叶的顺序。〔packages/coding-agent/src/core/session-manager.ts:408-415〕
- 因此 `parentId` 为空的 entry 是根；`buildSessionPath` 只沿 `current.parentId` 上溯，源码里没有环检测。**待验证**〔packages/coding-agent/src/core/session-manager.ts:408-415〕
- `SessionTreeNode` 是 `getTree()` 的防御性拷贝：`entry`、`children`、`label?`、`labelTimestamp?`（后者是该 entry 最近一次 label 变更的时间戳）。〔packages/coding-agent/src/core/session-manager.ts:199-207〕

## 5. 规范上下文的构建

- `buildSessionProjection` 是入口：先 `buildSessionPath` 取路径并由 `getSessionContextSettings(path)` 得出 `thinkingLevel` 与 `model`，再 `buildContextEntries` 取上下文条目，再收集 `context_edit` 做映射，最后 map 成 `ProjectedSessionEntry`。〔packages/coding-agent/src/core/session-manager.ts:542-556〕
- `ProjectedSessionEntry` 有 `sourceEntry`（拥有该投影贡献的原始追加 entry）与 `messages`（应用 context edit 之后的模型可见消息；状态型 entry 与被省略的条目为空数组）。〔packages/coding-agent/src/core/session-manager.ts:209-214〕
- `SessionProjection` 返回 `entries`、`messages`（`entries.flatMap(e => e.messages)`）、`thinkingLevel`、`model`；`model` 无变更时为 `null`。〔packages/coding-agent/src/core/session-manager.ts:216-221〕〔packages/coding-agent/src/core/session-manager.ts:567-572〕
- `buildSessionContext(entries, leafId?, byId?)` 只是 `buildSessionProjection` 的薄封装，返回 `{ messages, thinkingLevel, model }`。〔packages/coding-agent/src/core/session-manager.ts:575-583〕
- `buildContextEntries` 的 compaction 处理：沿路径取**最后一个** compaction entry 作为唯一 checkpoint；没有 compaction 就原样返回整条路径。〔packages/coding-agent/src/core/session-manager.ts:476-492〕
- 有 compaction 时上下文条目 = `[compaction]` + 从 `firstKeptEntryId` 起（不含）到 compaction 之前的条目（其中 role 为 system 的 message 被跳过）+ compaction 之后的所有条目；更早被摘要掉的条目被省略。〔packages/coding-agent/src/core/session-manager.ts:499-511〕
- 边界规则：`compaction` entry 在投影里只有位于 index 0 的那个会贡献 checkpoint 与 summary；若 index > 0 则 `messages: []`。注释解释 `buildContextEntries()` 可能因为旧 compaction 的原始 id 落在最新保留区间内而保留它。〔packages/coding-agent/src/core/session-manager.ts:557-565〕
- `CustomEntry` 不进上下文（`buildSessionContext` 忽略），`CustomMessageEntry` 会进且被转成 user message。〔packages/coding-agent/src/core/session-manager.ts:125-126〕〔packages/coding-agent/src/core/session-manager.ts:151-153〕

## 6. 版本号与迁移

- 会话版本迁移在 `session-manager.ts` 内部，与 `migrations.ts` 的启动期迁移不是一回事：后者的导出只有 `migrateAuthToAuthJson`、`migrateSessionsFromAgentRoot`、`runMigrations`。〔packages/coding-agent/src/migrations.ts:21-21〕〔packages/coding-agent/src/migrations.ts:84-84〕〔packages/coding-agent/src/migrations.ts:305-305〕
- `migrateToCurrentVersion` 读 header 的 `version`（缺省视为 1），若已 >= 3 直接返回 false；否则按序执行 `version < 2` 的 v1→v2 与 `version < 3` 的 v2→v3，返回是否应用过迁移。**原地修改** entries。〔packages/coding-agent/src/core/session-manager.ts:333-347〕
- v1→v2 的内容是补 `id`/`parentId` 树结构：逐条 entry 生成不冲突的 id、`parentId` 指向前一条（`prevId`），并把 compaction 的 `firstKeptEntryIndex` 换算成 `firstKeptEntryId` 后删除原字段；header 的 `version` 置 2。〔packages/coding-agent/src/core/session-manager.ts:286-313〕
- v2→v3 的内容是把 message entry 里 role 为 `hookMessage` 的消息改成 `custom`；header 的 `version` 置 3。〔packages/coding-agent/src/core/session-manager.ts:315-331〕
- id 生成用 `generateId(ids)`，冲突时回退到 `randomUUID()`。〔packages/coding-agent/src/core/session-manager.ts:280-283〕
- 加载时对文件不做 schema 校验，注释写明 session 文件可能被 `hand-edited`，因此对 `null`/缺失 content 的消息做了兼容处理。〔packages/coding-agent/src/core/session-manager.ts:439-446〕

## 7. 读写方式

- 读取 `loadEntriesFromFile` 用 `StringDecoder("utf8")` + 定长 `Buffer.allocUnsafe(SESSION_READ_BUFFER_SIZE)`（1 MB）逐块 `readSync`，按 `\n` 切行，因此**跨网络块/跨读缓冲的多字节字符与被切断的行都能正确还原**。〔packages/coding-agent/src/core/session-manager.ts:627-651〕
- 空行与 JSON 解析失败的行被静默跳过（`parseSessionEntryLine` 对畸形行返回 null）；文件末尾没有换行时最后一段残留也会被单独解析一次。〔packages/coding-agent/src/core/session-manager.ts:616-624〕〔packages/coding-agent/src/core/session-manager.ts:654-656〕
- 读完会校验首条必须是 `type === "session"` 且 `id` 是字符串，否则整份返回空数组（视为损坏，不尝试修复）。〔packages/coding-agent/src/core/session-manager.ts:661-666〕
- 若读完后仍有残留（即文件最后一行没有换行符），会 `appendFileSync(resolvedFilePath, "\n")` 补一个换行修好文件。〔packages/coding-agent/src/core/session-manager.ts:668-668〕
- header 单独扫描走 `readSessionHeader`，缓冲 4096 字节，上限 `MAX_SESSION_HEADER_SCAN_BYTES` = 1 MB，超限抛 `Session header exceeds ...-byte scan limit`，目的注释是"限制同步 header 查找同时允许很长的 cwd 与自定义元数据字段"。〔packages/coding-agent/src/core/session-manager.ts:604-614〕〔packages/coding-agent/src/core/session-manager.ts:685-700〕
- 写入是"延迟 flush"策略：`_persist` 先检查 `fileEntries` 里有没有 assistant message；没有就不写盘，只把 `flushed` 置 false，等 assistant 到来时把累积的全部 entry 一次性写入。〔packages/coding-agent/src/core/session-manager.ts:1160-1172〕
- 首次 flush 用 `openSync(this.sessionFile, "wx")`（独占创建，文件已存在会失败），逐条 `writeFileSync(fd, \`${JSON.stringify(e)}\n\`)`；之后每条 entry 走 `appendFileSync`。〔packages/coding-agent/src/core/session-manager.ts:1174-1186〕
- 整份重写 `_rewriteFile` 用 `openSync(this.sessionFile, "w")` 截断后逐条写回全部 `fileEntries`；仅在 `persist` 且有 `sessionFile` 时生效。〔packages/coding-agent/src/core/session-manager.ts:1124-1134〕
- 带目标 cwd 的 fork：先算 `newSessionId` 与 `fileTimestamp`，文件名同样是 `${fileTimestamp}_${newSessionId}.jsonl`；新 header 的 `parentSession` 指向源会话路径、`cwd` 为目标 cwd，用 `flag: "wx"` 写第一行，然后把源会话里所有非 header entry 逐条 `appendFileSync` 过去。〔packages/coding-agent/src/core/session-manager.ts:1839-1859〕

## 8. 启动期一次性迁移（`migrations.ts`）

- `migrateAuthToAuthJson`：若 `auth.json` 已存在则直接返回空数组；否则把旧 `oauth.json` 逐 provider 转成 `{ type: "oauth", ...cred }` 并把旧文件 `renameSync` 成 `.migrated`，再把 `settings.json` 里的 `apiKeys` 转成 `{ type: "api_key", key }` 并从 settings 中 `delete`；最终以 `mode: 0o600` 写 `auth.json`。〔packages/coding-agent/src/migrations.ts:21-73〕
- 每一步的 JSON 解析都在 try/catch 里，出错则跳过该步（注释 `Skip on error`）。〔packages/coding-agent/src/migrations.ts:42-44〕
- `migrateSessionsFromAgentRoot` 修的是 v0.30.0 的 bug：会话曾被写到 `~/.pi/agent/` 根目录而非 `~/.pi/agent/sessions/<编码 cwd>/`，该迁移按 session header 里的 cwd 把它们搬到正确位置。〔packages/coding-agent/src/migrations.ts:75-83〕

## 未覆盖 / 不确定

- **部分证实**（P4）：`sessionEntryToContextMessages` 对 `message` 条目的规则已实现并对拍——系统消息内容为空时丢弃、`user`/`assistant` 的 `content` 为 `null` 时规范成 `[]`，其余原样返回〔packages/coding-agent/src/core/session-manager.ts:439-452〕
- **已验证**（P4b）：`branch_summary` 的 `summary` 会变成一条 **user** 消息（`convertToLlm` 用 `BRANCH_SUMMARY_PREFIX` + summary 拼接），本项目**尚未建模**该条目类型→`--continue` 会少这条〔packages/coding-agent/src/core/messages.ts:170-176〕
- **已验证**（P4b）：`custom_message` 的 `content` 同样变成一条 **user** 消息（字符串则包成单个 text 块），本项目**尚未建模**该条目类型〔packages/coding-agent/src/core/messages.ts:163-169〕
- `SessionInfo` 的字段是 `path`、`id`、`cwd`、`name?`、`parentSessionPath?`（`created` 等见源码）。〔packages/coding-agent/src/core/session-manager.ts:229-240〕
- 对应的条目类型是 `SessionInfoEntry`：`type: "session_info"` 加可选 `name`。〔packages/coding-agent/src/core/session-manager.ts:142-145〕
- 会话树对外由 `getTree()` 给出（节点 `{entry, children}`、根是 `parentId` 为空的条目）。〔packages/coding-agent/src/core/session-manager.ts:1522-1535〕
- 导航入口是 `navigateTree`（print/rpc/interactive 三个模式各自转发到它）。〔packages/coding-agent/src/core/agent-session.ts:3581-3585〕
- `--resume` 走**交互选择**：`selectSession` 用 `SessionManager.list`（项目内）与 `SessionManager.listAll`（全部）两个来源；没选到就打印 No session selected 并以 0 退出。〔packages/coding-agent/src/main.ts:414-424〕
- `buildSessionPath` 的循环只有 `while (current) { path.push(current); current = current.parentId ? index.get(current.parentId) : undefined; }`——**没有任何环检测或访问计数**，因此文件里 `parentId` 若成环会**死循环**（已核实，未发现别处防御）。〔packages/coding-agent/src/core/session-manager.ts:408-415〕
- 路径上的上下文设置取**最后一条**：`thinking_level_change` 给 `thinkingLevel`，`model_change` 给 `model`，assistant 消息则用其自身的 `provider`/`modelId`。〔packages/coding-agent/src/core/session-manager.ts:418-432〕
- `runMigrations(cwd)` 的执行顺序：`migrateAuthToAuthJson()` → `migrateSessionsFromAgentRoot()` → `migrateToolsToBin()` → `migrateKeybindingsConfigFile()` → `migrateExtensionSystem(cwd)`；返回 `{migratedAuthProviders, deprecationWarnings}`。〔packages/coding-agent/src/migrations.ts:305-315〕
  - 其中 `migrateSessionsFromAgentRoot` 定义在同文件 `:84`，由 `runMigrations` 直接调用。〔packages/coding-agent/src/migrations.ts:305-310〕
- fork 时旧分支条目的 `parentId` **会被重接**：`createBranchedSession` 里除 `label` 外的每条路径条目都写成 `{...entry, parentId: pathParentId}`（`compaction` 条目另处理 `firstKeptEntryId`）。〔packages/coding-agent/src/core/session-manager.ts:1625-1660〕
- 环境变量名是拼出来的：`ENV_SESSION_DIR` = `${APP_NAME.toUpperCase()}_CODING_AGENT_SESSION_DIR`，即 `PI_CODING_AGENT_SESSION_DIR`。〔packages/coding-agent/src/config.ts:509-509〕
- session 目录的优先级是一条 `??` 链：`parsed.sessionDir`（经 `normalizePath`）> 环境变量（经 `expandTildePath`）> `startupSettingsManager.getSessionDir()`。〔packages/coding-agent/src/main.ts:675-679〕
- `--session-dir` 的解析就是 `result.sessionDir = args[++i]`。〔packages/coding-agent/src/cli/args.ts:140-140〕
- `_setSessionFile` 在设置会话文件时会调用 `_rewriteFile`（其上有 `statSync(explicitPath)` 与 `size` 判断）。〔packages/coding-agent/src/core/session-manager.ts:1029-1045〕
- `_loadEntries` 只在 `migrateToCurrentVersion` 返回真时调用 `_rewriteFile`——也就是**版本迁移**会整文件重写。〔packages/coding-agent/src/core/session-manager.ts:1085-1095〕
- `createBranchedSession` 在 `hasAssistant` 为真时调用 `_rewriteFile`（同段注释提到 no-assistant guard 之后会重置 `flushed`）。〔packages/coding-agent/src/core/session-manager.ts:1700-1720〕
- 结论：`_rewriteFile` 的调用点只有上述三处；**追加 `label` 或 `session_info` 条目本身不触发文件重写**。〔packages/coding-agent/src/core/session-manager.ts:1124-1124〕
- **已验证**（P4）：实际录制的 JSONL 行序与字段与本卡一致——45 个场景的 `session.jsonl` 与 golden **逐行一致**（A 层：行数必须相等，只放开 `id`/`parentId`/`targetId`/`timestamp`）；写会话用 `appendFileSync` + `JSON.stringify(entry)`，键序即结构体字段序〔packages/coding-agent/src/core/session-manager.ts:1166〕
- **仍未证实**：`parentSession` 在 fork 时的取值（S15 未实现）〔packages/coding-agent/src/core/session-manager.ts:49-54〕
