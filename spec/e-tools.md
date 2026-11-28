# spec/e-tools.md — read / write / edit / bash

来源 `packages/coding-agent/src/core/tools/{read,write,edit,bash,edit-diff,truncate,output-accumulator}.ts`。
四个工具都是 typebox schema + `ToolDefinition`，并通过 `wrapToolDefinition` 包成运行时 `AgentTool`。

## 0. 公共约定

- 四个工具都声明 `constrainedSampling: { type: "json_schema", strict: "prefer" }`。〔packages/coding-agent/src/core/tools/read.ts:79-80〕〔packages/coding-agent/src/core/tools/write.ts:56-57〕〔packages/coding-agent/src/core/tools/edit.ts:155-156〕〔packages/coding-agent/src/core/tools/bash.ts:242-243〕
- 每个工具都定义一个 `*ToolSystemPromptContribution`，含 `snippet` 与 `guidelines`，其中 snippet 进工具的 `description` 辅助、guidelines 进系统提示；实现系统提示时必须照抄。〔packages/coding-agent/src/core/tools/read.ts:20-23〕〔packages/coding-agent/src/core/tools/write.ts:16-19〕〔packages/coding-agent/src/core/tools/edit.ts:43-51〕〔packages/coding-agent/src/core/tools/bash.ts:43-46〕
- 四个工具都有可插拔的 `*Operations` 接口（`readFile`/`access`/`writeFile`/`mkdir`/`exec`），默认实现是本地文件系统或本地 shell，可被扩展替换（例如走 SSH）。〔packages/coding-agent/src/core/tools/read.ts:31-48〕〔packages/coding-agent/src/core/tools/write.ts:23-37〕〔packages/coding-agent/src/core/tools/edit.ts:79-96〕〔packages/coding-agent/src/core/tools/bash.ts:55-78〕
- 路径解析：read 用异步的 `resolveReadPathAsync`，其余三个用同步的 `resolveToCwd`，且都以 `ctx?.cwd || cwd` 作为基准目录。〔packages/coding-agent/src/core/tools/read.ts:103-103〕〔packages/coding-agent/src/core/tools/write.ts:65-65〕〔packages/coding-agent/src/core/tools/edit.ts:161-161〕〔packages/coding-agent/src/core/tools/bash.ts:253-257〕
- write 与 edit 都在 `withFileMutationQueue` 里按绝对路径排队执行，保证同一路径的写操作串行。〔packages/coding-agent/src/core/tools/write.ts:67-67〕〔packages/coding-agent/src/core/tools/edit.ts:163-163〕

## 1. 截断常量

- `DEFAULT_MAX_LINES = 2000`，`DEFAULT_MAX_BYTES = 50 * 1024`（50KB），`GREP_MAX_LINE_LENGTH = 500`。〔packages/coding-agent/src/core/tools/truncate.ts:11-13〕
- 注释明确两个限制是独立的（`whichever is hit first wins`），并且 `Never returns partial lines`，唯一例外是 bash 的尾部截断边界情况。〔packages/coding-agent/src/core/tools/truncate.ts:1-9〕
- `TruncationResult` 的字段：`content`、`truncated`、`truncatedBy`（`"lines" | "bytes" | null`）、`totalLines`、`totalBytes`、`outputLines`、`outputBytes`、`lastLinePartial`、`firstLineExceedsLimit`、`maxLines`、`maxBytes`。〔packages/coding-agent/src/core/tools/truncate.ts:15-38〕
- `truncateHead` 与 `truncateTail` 都用 `options.maxLines ?? DEFAULT_MAX_LINES` 与 `options.maxBytes ?? DEFAULT_MAX_BYTES`；read 用 head，bash 用 tail。〔packages/coding-agent/src/core/tools/truncate.ts:78-80〕〔packages/coding-agent/src/core/tools/truncate.ts:168-170〕
- head 截断里"第一行本身就超字节上限"是独立分支：返回 `content: ""`、`truncatedBy: "bytes"`、`firstLineExceedsLimit: true`。〔packages/coding-agent/src/core/tools/truncate.ts:103-119〕

## 2. read

- 入参 schema：`path`（必填字符串，相对或绝对）、`offset`（可选数字，1 起算的行号）、`limit`（可选数字，最多读多少行）。〔packages/coding-agent/src/core/tools/read.ts:14-18〕
- 工具 description 里写明：文本输出截断到 `DEFAULT_MAX_LINES` 行或 `DEFAULT_MAX_BYTES / 1024` KB（先到先算），大文件用 offset/limit，需要全文就顺着 offset 继续读。〔packages/coding-agent/src/core/tools/read.ts:76-76〕
- `ReadToolDetails` 只有可选的 `truncation`；description 里列出的图片格式是 jpg、png、gif、webp、bmp，图片 `as attachments` 发送。〔packages/coding-agent/src/core/tools/read.ts:27-29〕〔packages/coding-agent/src/core/tools/read.ts:76-76〕
- 执行是包在 `new Promise` 里的：进入先检查 `signal?.aborted`，已 abort 立即 reject `Operation aborted`；随后注册 abort 监听器，同样 reject `Operation aborted`。〔packages/coding-agent/src/core/tools/read.ts:88-99〕
- 顺序是 `resolveReadPathAsync` → `ops.access`（检查存在与可读）→ `detectImageMimeType`。〔packages/coding-agent/src/core/tools/read.ts:103-108〕
- 图片分支：`processImage(buffer, mimeType, { autoResizeImages, resizeOptions })`，resize 配置取 `ctx?.model?.inputLimits?.images?.resize ?? fallbackResizeOptions`；失败时只返回文本 `Read image file [${mimeType}]\n${processed.message}`。〔packages/coding-agent/src/core/tools/read.ts:112-122〕
- 图片成功时返回两条内容：先文本 `Read image file [${processed.mimeType}]`（可附 hints 与非视觉模型提示），再 image 块。〔packages/coding-agent/src/core/tools/read.ts:123-131〕
- 已验证（Anthropic provider 场景）：`toolResult` 的 content 确实是 `['text','image']`，文本为 `Read image file [image/png]`，图片块带 base64 数据。〔golden/S25-anthropic/session.jsonl:7-7〕
- 已验证（大文件 read 的证据，对应 Q4）：**S21** 让模型读一个约 36 KB / 1200 行的大文件（夹具 `fixtures/sse/openai/tool-read-big.sse`），会话里**有** `toolResult` 条目，实测 42640 字节、`toolName` 为 `read`。此前"大文件 read 不产生 toolResult"的观察是我生成夹具时的缺陷（tool-call 行被当作原始字符串传给 `extend`，被逐字符拆开），**该结论已撤回**；因此"read 截断只有小文件证据"的说法不再成立。〔golden/S21/session.jsonl:7-7〕
- 已验证：OpenAI provider 场景下同样产生含 `image` 块的 toolResult。〔golden/S25-openai/session.jsonl:7-7〕
- 非视觉模型提示文案是 `[Current model does not support images. The image will be omitted from this request.]`，仅当 `ctx.model` 存在且 `model.input` 不含 `"image"` 时追加。〔packages/coding-agent/src/core/tools/read.ts:59-64〕
- 文本分支先按 `\n` 切成行数组，`offset` 从 1 基转 0 基：`startLine = offset ? Math.max(0, offset - 1) : 0`。〔packages/coding-agent/src/core/tools/read.ts:136-140〕
- `offset` 越界的错误文本：`Offset ${offset} is beyond end of file (${allLines.length} lines total)`。〔packages/coding-agent/src/core/tools/read.ts:141-144〕
- `limit` 优先：给了 `limit` 就先切 `allLines.slice(startLine, Math.min(startLine + limit, allLines.length))`，否则取 `slice(startLine)`；之后**无论如何**都再过一遍 `truncateHead`。〔packages/coding-agent/src/core/tools/read.ts:145-156〕
- 第一行就超限时不给内容，而是提示用 bash 兜底：`[Line ${startLineDisplay} is ${firstLineSize}, exceeds ${formatSize(DEFAULT_MAX_BYTES)} limit. Use bash: sed -n '${startLineDisplay}p' ${path} | head -c ${DEFAULT_MAX_BYTES}]`，并把 `truncation` 放进 details。〔packages/coding-agent/src/core/tools/read.ts:158-163〕
- 普通截断时按 `truncatedBy` 给两种续读提示：按行截断是 `[Showing lines ${startLineDisplay}-${endLineDisplay} of ${totalFileLines}. Use offset=${nextOffset} to continue.]`，按字节截断则在中间多 `(50KB limit)` 一段。〔packages/coding-agent/src/core/tools/read.ts:163-173〕
- 没截断但用户的 `limit` 提前截断且文件还有内容时，提示 `[${remaining} more lines in file. Use offset=${nextOffset} to continue.]`。〔packages/coding-agent/src/core/tools/read.ts:174-178〕
- 四种情况的判定顺序是：`firstLineExceedsLimit` → `truncation.truncated` → `userLimitedLines !== undefined` → 原样输出。〔packages/coding-agent/src/core/tools/read.ts:158-182〕
- abort 处理：每个 await 之后都检查内部 `aborted` 标志直接 return，成功与异常路径都会 `removeEventListener`；已 abort 时不再 reject。〔packages/coding-agent/src/core/tools/read.ts:186-192〕

## 3. write

- 入参 schema 只有两个必填字符串：`path` 与 `content`。〔packages/coding-agent/src/core/tools/write.ts:11-14〕
- description 写明 `Creates the file if it doesn't exist, overwrites if it does`，并 `Automatically creates parent directories`。〔packages/coding-agent/src/core/tools/write.ts:52-53〕
- 执行体在文件变更队列内，顺序是 `throwIfAborted()` → `ops.mkdir(dirname(absolutePath))` → `throwIfAborted()` → `ops.writeFile(absolutePath, content)` → `throwIfAborted()`。〔packages/coding-agent/src/core/tools/write.ts:67-83〕
- 注释解释为什么不在 abort 事件监听器里 reject：那会在 in-flight 文件操作还没结束时就把变更队列放掉；改为每个 await 之后检查 `signal.aborted`，既观察到同样的取消，又保证队列锁到当前操作 settle。〔packages/coding-agent/src/core/tools/write.ts:68-74〕
- 成功文本是 `Successfully wrote to ${path}`（用原始入参 path，不是绝对路径），`details` 为 undefined。〔packages/coding-agent/src/core/tools/write.ts:85-88〕
- 默认写文件用 `fsWriteFile(path, content, "utf-8")`，默认建目录用 `fsMkdir(dir, { recursive: true })`。〔packages/coding-agent/src/core/tools/write.ts:34-37〕

## 4. edit

- 入参 schema：`path` 必填字符串，加 `edits` 必填数组，数组元素是 `{ oldText, newText }` 两个必填字符串。〔packages/coding-agent/src/core/tools/edit.ts:21-41〕
- `oldText` 的 description 明确要求：在原文件中唯一，且不得与同一次调用里其他 `edits[].oldText` 重叠。〔packages/coding-agent/src/core/tools/edit.ts:23-26〕
- `edits` 数组的 description 明确要求：每条 edit 都针对**原始文件**匹配，不是增量匹配；不要给出重叠或嵌套的 edit；两处改动靠近同一块时应合并成一条。〔packages/coding-agent/src/core/tools/edit.ts:35-38〕
- `prepareEditArguments` 做兼容归一：某些模型（注释点名 Opus 4.6、GLM-5.1）把 `edits` 发成 JSON 字符串，或发成单个 edit 对象而非一元素数组，这两种都会被纠正；还支持顶层遗留的 `oldText`/`newText`，会追加进 `edits`。〔packages/coding-agent/src/core/tools/edit.ts:103-134〕
- `validateEditInput` 的错误文本：`Edit tool input is invalid. edits must contain at least one replacement.`（`edits` 不是数组或为空数组时抛）。〔packages/coding-agent/src/core/tools/edit.ts:136-141〕
- 执行顺序：`resolveToCwd` → 进 `withFileMutationQueue` → `throwIfAborted` → `ops.access`（可读可写）。〔packages/coding-agent/src/core/tools/edit.ts:159-177〕
- 文件不可访问时错误文本为 `Could not edit file: ${path}. ${errorMessage}.`，其中 `errorMessage` 是 `Error code: ${error.code}` 或 `String(error)`。〔packages/coding-agent/src/core/tools/edit.ts:177-182〕
- 读文件后先 `splitBom` 去掉 BOM（注释理由：模型不会在 `oldText` 里包含不可见的 BOM），再 `detectLineEnding` 记录原换行符、`normalizeToLF` 归一，最后把新内容 `restoreLineEndings` 回原换行并把 BOM 加回去。〔packages/coding-agent/src/core/tools/edit.ts:190-198〕
- 应用编辑的函数是 `applyEditsToNormalizedContent(normalizedContent, edits, path)`，先对每个 `oldText`/`newText` 做 `normalizeToLF`。〔packages/coding-agent/src/core/tools/edit.ts:194-194〕〔packages/coding-agent/src/core/tools/edit-diff.ts:300-308〕
- 匹配用 `fuzzyFindText`：先精确 `indexOf`，找不到再在 `normalizeForFuzzyMatch` 归一后的空间里找；fuzzy 命中时返回的偏移是**归一空间**的偏移。〔packages/coding-agent/src/core/tools/edit-diff.ts:207-233〕
- 只要有任意一条 edit 走了 fuzzy，就把整个基准内容 `normalizeForFuzzyMatch` 后再匹配，最后用 `applyReplacementsPreservingUnchangedLines` 保留未改动行的原样；全精确则用 `applyReplacements`。〔packages/coding-agent/src/core/tools/edit-diff.ts:316-318〕〔packages/coding-agent/src/core/tools/edit-diff.ts:352-355〕
- 校验顺序：任一 `oldText` 为空 → `getEmptyOldTextError`；找不到 → `getNotFoundError`；出现次数 > 1 → `getDuplicateError`；按 `matchIndex` 排序后相邻区间重叠 → 抛 overlap 错误。〔packages/coding-agent/src/core/tools/edit-diff.ts:310-314〕〔packages/coding-agent/src/core/tools/edit-diff.ts:323-331〕〔packages/coding-agent/src/core/tools/edit-diff.ts:341-350〕
- `oldText` 为空的错误文本：单条时 `oldText must not be empty in ${path}.`，多条时 `edits[${editIndex}].oldText must not be empty in ${path}.`。〔packages/coding-agent/src/core/tools/edit-diff.ts:275-280〕
- 找不到的错误文本：单条时 `Could not find the exact text in ${path}. The old text must match exactly including all whitespace and newlines.`；多条时 `Could not find edits[${editIndex}] in ${path}. The oldText must match exactly including all whitespace and newlines.`〔packages/coding-agent/src/core/tools/edit-diff.ts:253-262〕
- 不唯一的错误文本：单条时 `Found ${occurrences} occurrences of the text in ${path}. The text must be unique. Please provide more context to make it unique.`；多条时 `Found ${occurrences} occurrences of edits[${editIndex}] in ${path}. Each oldText must be unique. Please provide more context to make it unique.`〔packages/coding-agent/src/core/tools/edit-diff.ts:264-273〕
- 重叠的错误文本：`edits[${previous.editIndex}] and edits[${current.editIndex}] overlap in ${path}. Merge them into one edit or target disjoint regions.`〔packages/coding-agent/src/core/tools/edit-diff.ts:346-348〕
- 替换后内容与原内容完全相同时（`baseContent === newContent`）抛 no-change 错误，单条文本以 `No changes made to` 开头。〔packages/coding-agent/src/core/tools/edit-diff.ts:282-287〕〔packages/coding-agent/src/core/tools/edit-diff.ts:357-359〕
- 成功文本是 `Successfully replaced ${edits.length} block(s) in ${path}.`；`EditToolDetails` 带 `diff`（展示用）、`patch`（标准 unified patch）、可选 `firstChangedLine`。〔packages/coding-agent/src/core/tools/edit.ts:203-211〕〔packages/coding-agent/src/core/tools/edit.ts:70-77〕

## 5. bash

- 入参 schema：`command` 必填字符串，`timeout` 可选数字（**秒**，无默认值即不超时）。〔packages/coding-agent/src/core/tools/bash.ts:38-41〕
- description 说明返回 stdout 与 stderr，输出截断到**最后** `DEFAULT_MAX_LINES` 行或 50KB（先到先算），被截断时完整输出存到临时文件。〔packages/coding-agent/src/core/tools/bash.ts:239-239〕
- `MAX_TIMEOUT_MS = 2_147_483_647`，`MAX_TIMEOUT_SECONDS` 由它除以 1000 得到。〔packages/coding-agent/src/core/tools/bash.ts:22-23〕
- `resolveTimeoutMs` 的校验：非有限数或 <= 0 抛 `Invalid timeout: must be a finite number of seconds`；超过上限抛 `Invalid timeout: maximum is ${MAX_TIMEOUT_SECONDS} seconds`。〔packages/coding-agent/src/core/tools/bash.ts:25-36〕
- 工作目录不存在时抛 `Working directory does not exist: ${cwd}\nCannot execute ${shellName} commands.`。〔packages/coding-agent/src/core/tools/bash.ts:89-93〕
- spawn 时 `detached: process.platform !== "win32"`，stdin 是 `"pipe"`（当 shell 的 commandTransport 为 stdin 时把命令写进 stdin 并 end）或 `"ignore"`；子进程 pid 会被 `trackDetachedChildPid` 登记。〔packages/coding-agent/src/core/tools/bash.ts:95-107〕
- 超时与 abort 的清理都走 `killProcessTree(child.pid)`；超时定时器到点时先把 `timedOut = true` 再杀。〔packages/coding-agent/src/core/tools/bash.ts:108-121〕
- abort 监听在 spawn 之后注册，且注册前先检查 `signal.aborted` 立即执行一次 `onAbort`。〔packages/coding-agent/src/core/tools/bash.ts:125-129〕
- 退出码约定：spawn 出错时用 `waitForChildProcess(child)` 等进程真正结束（注释：避免被 detached 后代持有的 stdio 句柄挂住）；被信号杀死时 shell 没有退出码，用 shell 惯例 `128 + (osConstants.signals[signalCode] ?? 0)`；`exitCode` 为 null 且无信号时按 1 处理。〔packages/coding-agent/src/core/tools/bash.ts:130-142〕
- `finally` 里无条件清理：取消 detached pid 登记、清 timeout 定时器、移除 abort 监听。〔packages/coding-agent/src/core/tools/bash.ts:143-147〕
- 工具层把内部错误翻译成用户可见文本：abort 追加 `Command aborted`；`timeout:` 前缀的错误取冒号后的秒数，追加 `Command timed out after ${timeoutSecs} seconds`；两者都是把已捕获的输出放在前面、状态放在后面。〔packages/coding-agent/src/core/tools/bash.ts:341-364〕
- 退出码为 null 追加 `Command terminated without an exit code`；非 0 追加 `Command exited with code ${exitCode}`；两者都作为错误抛出（抛错而非返回 isError）。〔packages/coding-agent/src/core/tools/bash.ts:366-374〕
- `formatOutput` 的 `emptyText` 默认值是 `(no output)`；但 abort/timeout 分支显式传空串占位。〔packages/coding-agent/src/core/tools/bash.ts:321-323〕〔packages/coding-agent/src/core/tools/bash.ts:354-355〕
- 大输出处理：输出累积器用 `truncateTail`（保留尾部），`snapshot({ persistIfTruncated: true })` 在发生截断时把完整输出落盘并回填 `fullOutputPath`。〔packages/coding-agent/src/core/tools/output-accumulator.ts:91-120〕
- 截断提示有三种形态：末行被部分截断时 `[Showing last ${formatSize(truncation.outputBytes)} of line ${endLine} (line is ${lastLineSize}). Full output: ${path}]`；按行截断 `[Showing lines ${startLine}-${endLine} of ${truncation.totalLines}. Full output: ${path}]`；按字节截断则在中间加 `(${formatSize(DEFAULT_MAX_BYTES)} limit)`。〔packages/coding-agent/src/core/tools/bash.ts:325-337〕
- `BashToolDetails` 带可选 `truncation` 与 `fullOutputPath`；临时文件名前缀默认 `pi-bash`。〔packages/coding-agent/src/core/tools/bash.ts:50-53〕〔packages/coding-agent/src/core/tools/bash.ts:383-391〕
- 流式回显走节流：`scheduleOutputUpdate` 按 `BASH_UPDATE_THROTTLE_MS - (Date.now() - lastUpdateAt)` 合并更新，脏标记为 false 时直接跳过。〔packages/coding-agent/src/core/tools/bash.ts:265-299〕
- 环境变量处理：先取 `getShellEnv()` 并**无条件删除** `PI_SESSION_ID`、`PI_SESSION_FILE`、`PI_PROVIDER`、`PI_MODEL`、`PI_REASONING_LEVEL`，只有 `exposeSessionEnvironment`（默认 true）且有 ctx 时才重新写入这些变量。〔packages/coding-agent/src/core/tools/bash.ts:170-193〕

## 未覆盖 / 不确定

- 【已验证】`normalizeForFuzzyMatch` / `applyReplacements` / `applyReplacementsPreservingUnchangedLines` 的归一化与替换语义：用 `tools/oracle/edit-oracle.mjs` 调 TS 真实的 `createEditToolDefinition` 跑 40 组边界用例（重复 / 部分行覆盖 / CRLF / 空文件 / 多编辑组合…），Rust 侧逐字节比对改后文件内容，38/40 一致（见 `crates/pi-core/tests/t3_23_edit_oracle.rs` 与 `fixtures/edit/cases.json`）。**唯一已知差异是 NFKC**：TS 第一步做 `text.normalize("NFKC")`（全角 `Ａ` → 半角 `A`），Rust 未实现。此前担心的"整行粒度 vs 字节偏移"经此验证**不存在差异**。〔packages/coding-agent/src/core/tools/edit-diff.ts:11-320〕
- `truncateHead` 的快速路径：`totalLines <= maxLines && totalBytes <= maxBytes` 时直接返回原文（`truncated: false`）。〔packages/coding-agent/src/core/tools/truncate.ts:84-101〕
- **首行单独**就超过 `maxBytes` 时返回空内容，并置 `truncatedBy: "bytes"` 与 `firstLineExceedsLimit: true`。〔packages/coding-agent/src/core/tools/truncate.ts:102-118〕
- 其余情况按**行**循环：循环条件同时受 `maxLines` 限制，循环体内累计每行 `Buffer.byteLength`（除首行外各加 1 个换行）；一旦累计超过 `maxBytes` 就置 `truncatedBy = "bytes"` 并跳出，否则保持默认 `"lines"`——即**行数先判定、字节在行内判定**。〔packages/coding-agent/src/core/tools/truncate.ts:126-138〕
- `killProcessTree(pid)` 在 win32 上走 `taskkill`（参数 `/F`、`/T`、`/PID`，以 `detached: true` 启动）。〔packages/coding-agent/src/utils/shell.ts:216-230〕
- 非 Windows 分支用 `process.kill(-pid, "SIGKILL")`——**负 pid 表示整个进程组**；组杀失败时退回 `process.kill(pid, "SIGKILL")` 只杀该子进程。〔packages/coding-agent/src/utils/shell.ts:235-244〕
- `getShellConfig` 的顺序：先看自定义 shell 路径（不存在则抛 `Custom shell path not found:`），再按平台查找（win32 依次探 `ProgramFiles`、`ProgramFiles(x86)` 下的 git-bash，用 `existsSync` 校验）。〔packages/coding-agent/src/utils/shell.ts:66-90〕
- `getBashShellConfig` 决定 args：legacy WSL 的 bash 用 `["-s"]` 且 `commandTransport: "stdin"`，其余用 `["-c"]`。〔packages/coding-agent/src/utils/shell.ts:20-22〕
- 分离子进程的登记/注销是 `trackDetachedChildPid` 与 `untrackDetachedChildPid`；`waitForChildProcess` 不在本文件（未在本卡定位）。〔packages/coding-agent/src/utils/shell.ts:198-204〕
- `resolveToCwd` 就是 `resolvePath` 的薄封装——即“相对 cwd 归一化”的基准实现。〔packages/coding-agent/src/core/tools/path-utils.ts:48-49〕
- `resolveReadPath` 在 `resolveToCwd` 的结果上**先试原路径**（`fileExists` 命中即返回），不命中再试 am/pm、NFD、curly 等名字变体——差异在于**多了变体回退**，不是“不存在就报错”。〔packages/coding-agent/src/core/tools/path-utils.ts:52-56〕
- 异步版 `resolveReadPathAsync` 同样以 `resolveToCwd` 为起点再试变体。〔packages/coding-agent/src/core/tools/path-utils.ts:86-108〕
- `wrapToolDefinition` 把 `ToolDefinition` **投影**成 AgentTool（`name`、`parameters` 等字段照搬），`wrapToolDefinitions` 对列表做同样的事。〔packages/coding-agent/src/core/tools/tool-definition-wrapper.ts:5-31〕
- `createToolDefinitionFromAgentTool` 是**反向**投影；注释说明它给出的是不含 prompt 元数据与 renderer 的普通 `AgentTool` 覆盖。〔packages/coding-agent/src/core/tools/tool-definition-wrapper.ts:33-47〕
- 真正的“工具抛错 → 工具结果”转换**不在这里**：在 agent 侧的 `executeToolCall`（注释写明 converting expected tool throws to error output），它把异常转成 `createErrorToolResult(...)` 且 `isError: true`；准备期失败走 `immediateError`。〔packages/agent/src/harness/execution/tools.ts:125-152〕
- ref 的 `core/tools/` 里除四个内置（`bash`/`edit`/`read`/`write`）外**确实还有** `find`、`grep`、`ls`、`powershell`：`index.ts` 分别从 `./find.ts`、`./grep.ts`、`./ls.ts`、`./powershell.ts` 导出各自的 Details 类型。〔packages/coding-agent/src/core/tools/index.ts:25-55〕
- 本项目的范围：四个默认内置工具（bash/edit/read/write）之外，`ls` 已实现（与 `powershell` 一样放在可选集里，需 `--tools ls`）；`find`/`grep` 仍未做，所以下面这四条事实针对四个默认工具在本卡里属**“不适用”**（ref 有实现，但不在本项目的规格范围内），而不是“未抽”。〔packages/coding-agent/src/core/tools/index.ts:25-28〕
### grep / find 工具（只读，委托外部 rg / fd）

- 两者都是只读的 `grep` / `find` 检索工具，默认激活集不含它们，需显式选中（上游同样不在默认集内）。〔packages/coding-agent/src/core/tools/grep.ts:29-39〕
- grep 参数：`pattern`（必填）、`path`、`glob`、`ignoreCase`、`literal`、`context`、`limit`（默认 100）。〔packages/coding-agent/src/core/tools/grep.ts:29-39〕
- grep 通过 `rg` 执行，固定使用 `--json`、`--line-number`、`--color=never`、`--hidden`，并在模式与路径前加 `--`。〔packages/coding-agent/src/core/tools/grep.ts:162-166〕
- grep 输出行是 `相对路径:行号: 该行文本`；无匹配时返回 `No matches found`；达到上限时追加 `matches limit reached` 提示（并建议用两倍 limit 重试）。〔packages/coding-agent/src/core/tools/grep.ts:255-300〕
- find 参数：`pattern`（glob，必填）、`path`、`limit`（默认 1000）。〔packages/coding-agent/src/core/tools/find.ts:26-33〕
- find 通过 `fd` 执行，始终带 `--glob`、`--color=never`、`--hidden` 与 `--max-results`；不在 git 仓库内时补 `--no-require-git`，否则 fd 会忽略 .gitignore。〔packages/coding-agent/src/core/tools/find.ts:182-200〕
- find 输出逐行相对路径；无结果时返回 `No files found matching pattern`；达到上限时追加 `results limit reached` 提示。〔packages/coding-agent/src/core/tools/find.ts:261-290〕
- 与上游的唯一差异：`rg` / `fd` 缺失时不尝试自动下载，直接返回带安装指引的错误；`fd` 探测顺序为 `PI_FD_BIN`、`fd`、`fdfind`。〔packages/coding-agent/src/core/tools/find.ts:172-177〕

### `powershell` 工具（仅 Windows，P8b 实现）

- 它是 bash 工具**同一套执行体的另一份配置**：`ShellToolConfig` 的 name/label 都是 `powershell`、`shellName` 是 `PowerShell`、UI 提示符 `PS>`、snippet 是 `Execute PowerShell commands`、临时文件前缀 `pi-powershell`。〔packages/coding-agent/src/core/tools/powershell.ts:47-53〕
- 执行入口复用 `createShellToolDefinition`（与 bash 同一条路径，含超时、进程树终止、输出截断）。〔packages/coding-agent/src/core/tools/bash.ts:227-246〕
- 命令前注入 UTF-8 输出前缀（`UTF8_OUTPUT_PREFIX`）：`try { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8 } catch {}`。〔packages/coding-agent/src/core/tools/powershell.ts:16〕
- shell 解析顺序 `pwsh.exe` → `powershell.exe`，参数 `-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command`。〔packages/coding-agent/src/utils/shell.ts:120-130〕
- 非 Windows 调用抛 `The powershell tool is only available on Windows.`（本实现同文案）。〔packages/coding-agent/src/utils/shell.ts:127〕
- **默认激活集不含它**（默认只有 `read`/`bash`/`edit`/`write` 四个），只能经 --tools powershell 或 settings 的 defaultTools 选中。〔packages/coding-agent/src/core/sdk.ts:258〕
- 工具表里它始终存在（`createAllTools` 里无条件构造），由激活集决定是否暴露给模型。〔packages/coding-agent/src/core/tools/index.ts:213-224〕

- 文件变更队列的粒度是**每个 key 一条 Promise 链**：`fileMutationQueues` 是 `Map<string, Promise<void>>`，另外 `registrationQueue` 把所有注册串起来。〔packages/coding-agent/src/core/tools/file-mutation-queue.ts:4-5〕
- key 由 `getMutationQueueKey` 求：先 `resolve(filePath)`，能取到 `realpath` 就用它（符号链接归一）；路径不存在时退回 resolved 绝对路径。〔packages/coding-agent/src/core/tools/file-mutation-queue.ts:16-31〕
- `withFileMutationQueue(filePath, fn)` 把本次调用接到该 key 的链尾（`currentQueue.then(() => nextQueue)`）——同一 key 严格按注册顺序串行，不同 key 互不阻塞。〔packages/coding-agent/src/core/tools/file-mutation-queue.ts:32-45〕
### `ls` 工具（P8b 实现）

- 参数只有两个可选项：`path`（默认当前目录）与 `limit`（默认 500）；没有递归开关。〔packages/coding-agent/src/core/tools/ls.ts:11-14〕
- 条目按名字大小写不敏感排序，目录加 `/` 后缀，包含点文件；无法 stat 的条目跳过。〔packages/coding-agent/src/core/tools/ls.ts:120-146〕
- 空目录返回 `(empty directory)`。〔packages/coding-agent/src/core/tools/ls.ts:135〕
- 到 limit 时追加 `entries limit reached` 提示（并建议 `limit=<2×limit>`）；字节截断时追加 `limit reached` 提示，两条用 `. ` 连接后包在 `[...]` 里。〔packages/coding-agent/src/core/tools/ls.ts:145-160〕
- 错误文案：`Path not found:`、`Not a directory:`、`Cannot read directory:` 后接绝对路径。〔packages/coding-agent/src/core/tools/ls.ts:89-104〕
