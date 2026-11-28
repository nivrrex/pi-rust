# spec/i-cli.md — CLI 参数、模式与退出码

来源 `packages/coding-agent/src/cli/args.ts`（解析与帮助）、
`packages/coding-agent/src/main.ts`（分派与退出码）、`packages/coding-agent/src/modes/print-mode.ts`（非交互模式）。

## 1. 解析结果形状

- `Mode` 只有三个值：`"text" | "json" | "rpc"`。〔packages/coding-agent/src/cli/args.ts:11-11〕
- `Args` 把所有可选项列成可选字段，另外有四个必填字段：`messages: string[]`、`fileArgs: string[]`、`unknownFlags: Map<string, boolean | string>`、`diagnostics: Array<{ type: "warning" | "error"; message: string }>`。〔packages/coding-agent/src/cli/args.ts:13-58〕
- `unknownFlags` 收集未识别的 `--flag`（注释说明可能是扩展 flag），带 `=` 的按 `key=value` 存，否则若下一个参数既不是 `-` 开头也不是 `@` 开头就当值吃掉，否则存 `true`。〔packages/coding-agent/src/cli/args.ts:237-250〕
- 单横线且未知的选项直接进 `diagnostics`，文本 `Unknown option: ${arg}`。〔packages/coding-agent/src/cli/args.ts:251-252〕
- 不以 `-` 开头的参数进 `messages`；以 `@` 开头的去掉 `@` 进 `fileArgs`。〔packages/coding-agent/src/cli/args.ts:235-236〕〔packages/coding-agent/src/cli/args.ts:253-255〕
- `--` 之后的一切都按位置参数处理：`@` 开头进 `fileArgs`，其余进 `messages`，然后停止解析。〔packages/coding-agent/src/cli/args.ts:82-90〕

## 2. 本项目范围内的参数

- 模型与凭据：`--provider`、`--model`、`--api-key`、`--thinking`（非法值只产生 warning，文本 `Invalid thinking level "...". Valid values: ...`）。〔packages/coding-agent/src/cli/args.ts:114-119〕〔packages/coding-agent/src/cli/args.ts:157-166〕
- 提示词：`--system-prompt`（覆盖）、`--append-system-prompt`（可重复，累加进数组）。〔packages/coding-agent/src/cli/args.ts:120-124〕
- 会话：`--continue`/`-c`、`--resume`/`-r`、`--no-session`、`--session`、`--session-id`、`--fork`、`--session-dir`、`--name`/`-n`（缺值报 error `--name requires a value`）。〔packages/coding-agent/src/cli/args.ts:110-113〕〔packages/coding-agent/src/cli/args.ts:125-140〕
- 工具选择：`--models`（逗号分隔后 trim）、`--no-tools`/`-nt`、`--no-builtin-tools`/`-nbt`、`--tools`/`-t`、`--exclude-tools`/`-xt`（后三者按逗号 split + trim + 过滤空串）。〔packages/coding-agent/src/cli/args.ts:141-156〕
- 资源加载：`--extension`/`-e`（可重复）、`--no-extensions`/`-ne`、`--skill`（可重复）、`--no-skills`/`-ns`、`--prompt-template`（可重复）、`--no-prompt-templates`/`-np`、`--theme`（可重复）、`--no-themes`、`--use-theme`、`--no-context-files`/`-nc`。〔packages/coding-agent/src/cli/args.ts:176-205〕
- 输出与模式：`--mode`、`--print`/`-p`、`--export`、`--verbose`。〔packages/coding-agent/src/cli/args.ts:95-109〕〔packages/coding-agent/src/cli/args.ts:167-175〕〔packages/coding-agent/src/cli/args.ts:227-228〕
- `--print`/`-p` 会顺手吃掉紧跟其后的一个位置参数，条件是它存在、不以 `@` 开头、且不以 `-` 开头或以 `---` 开头。〔packages/coding-agent/src/cli/args.ts:167-173〕
- TUI：`--tui-mode`，只接受 `regular` 与 `fullscreen`；缺值报 `--tui-mode requires regular or fullscreen`，非法值报 `Invalid TUI mode "...". Valid values: regular, fullscreen`。〔packages/coding-agent/src/cli/args.ts:213-226〕
- 离线：`--offline` 置 `offline: true`（等价 `PI_OFFLINE=1`，语义见 `spec/f-config.md` 第 1 节）。〔packages/coding-agent/src/cli/args.ts:233-234〕
- 项目信任：`--approve`/`-a` 置真、`--no-approve`/`-na` 置假。〔packages/coding-agent/src/cli/args.ts:229-232〕
- 帮助与版本：`--help`/`-h`、`--version`/`-v`；模型列表：`--list-models`（可带搜索模式）。〔packages/coding-agent/src/cli/args.ts:91-94〕〔packages/coding-agent/src/cli/args.ts:206-212〕
- `--mode` 缺值报 `--mode requires text, json, or rpc`，非法值报 `Invalid mode "...". Valid values: text, json, rpc`。〔packages/coding-agent/src/cli/args.ts:95-108〕
- 子命令 `pi install|remove|uninstall|update|list|config|auth` 由 `main.ts` 在参数解析前分派，不走 `parseArgs`。**待验证**〔packages/coding-agent/src/main.ts:132-160〕

## 3. 应用模式判定

- `toPrintOutputMode` 把 json 映射成 json、其余映射成 text。〔packages/coding-agent/src/main.ts:124-126〕
- rpc 模式的入口是 `runRpcMode(runtime)`，其返回类型是 `Promise<never>`，即进程不自行结束。〔packages/coding-agent/src/main.ts:932-932〕〔packages/coding-agent/src/modes/rpc/rpc-mode.ts:54-54〕
- `isPlainRuntimeMetadataCommand`：`--print` 未设、`--mode` 未设、且有 `--help` 或 `--list-models` 时成立，表示只需要运行期元数据而不必启动完整 runtime。〔packages/coding-agent/src/main.ts:128-130〕

## 4. 退出码

- `runPrintMode` 成功路径返回 `exitCode`，初值 0；只有最后一条消息是 assistant 且 `stopReason` 为 `"error"` 或 `"aborted"` 时才置 1，并在 stderr 打印 `errorMessage` 或 `Request ${stopReason}`。〔packages/coding-agent/src/modes/print-mode.ts:143-158〕
- text 模式的正文只输出 assistant 消息里 `type === "text"` 的块，逐块加换行写到 raw stdout。〔packages/coding-agent/src/modes/print-mode.ts:149-153〕
- `runPrintMode` 抛异常时打印 `message` 并返回 1。〔packages/coding-agent/src/modes/print-mode.ts:159-161〕
- main 只在 print 模式返回非 0 时设置 `process.exitCode`，随后统一 `process.exit(exitCode)`（注释说明更倾向用 `process.exit(0)` 以免一次性命令被坏扩展拖住）。〔packages/coding-agent/src/main.ts:976-979〕〔packages/coding-agent/src/main.ts:591-599〕
- `auth` 子命令：解析失败置 `process.exitCode = 1`；有未知选项也置 1，并把 diagnostics 拼成 `AuthCommandError` 抛出。〔packages/coding-agent/src/main.ts:139-160〕
- 另一处 `process.exitCode` 的三态映射是 `ready` → 0、`not_ready` → 1、其他 → 2。〔packages/coding-agent/src/main.ts:201-201〕
- 参数/前置条件类错误一律直接 `process.exit(1)`（**实测 `main.ts` 里共 16 处**，逐处归类：`--fork`/`--session-id` 的参数冲突、会话解析与"找不到会话"、会话 id 已存在、`--name` 空值、RPC 模式不支持 `fileArgs`、会话 cwd 缺失、启动基准仅限交互模式、无可用模型等；原文写的"共 8 处"我按实测改为 16，且未逐处标注旧编号）。〔packages/coding-agent/src/main.ts:309-916〕

## 5. 会话与配置相关的参数联动

- `--session` 显式路径被原样保留，不套用默认的 `<fileTimestamp>_<id>.jsonl` 命名（见 `spec/c-session.md` 第 1 节）。〔packages/coding-agent/src/core/session-manager.ts:1053-1053〕
- `--session-dir` 与 settings 的 `sessionDir` 同格式；`PI_CODING_AGENT_SESSION_DIR` 的优先级低于 `--session-dir`。**待验证**〔packages/coding-agent/src/cli/args.ts:139-140〕
- help 里打印的环境变量名由 `ENV_AGENT_DIR`、`ENV_SESSION_DIR` 与 `CONFIG_DIR_NAME` 拼出，不是硬编码字符串。〔packages/coding-agent/src/cli/args.ts:7-7〕〔packages/coding-agent/src/config.ts:507-509〕

## 未覆盖 / 不确定

- `printHelp` 定义在 cli/args.ts，帮助文本是一整段模板字符串（该文件共 458 行）；开头一行是 `APP_NAME` 加上 “- AI coding assistant with read, bash, edit, write tools”。〔packages/coding-agent/src/cli/args.ts:261-270〕
- 若传入 `extensionFlags`，前面会多一段 Extension CLI Flags 块：每行 `  --<name>[ <value>]` 左对齐到 30 列再加描述，描述缺省为 Registered by `<extensionPath>` 形式。〔packages/coding-agent/src/cli/args.ts:262-273〕
- 该模板里同时含选项段与命令段（例如 `--list-models` [search]、`uninstall`/`remove` 的说明行），因此“文本与顺序”以这一段为准。〔packages/coding-agent/src/cli/args.ts:280-323〕
  - 已核实部分：包管理类子命令在 `package-manager-cli.ts`——类型是 `PackageCommand`（`"install" | "remove" | "update" | "list"`），且 **`uninstall` 只是 `remove` 的别名**。〔packages/coding-agent/src/package-manager-cli.ts:46-46〕
  - 其分派（`case`）与每个命令的用法文本（`getPackageCommandUsage`）都在同一文件里。〔packages/coding-agent/src/package-manager-cli.ts:267-273〕
  - `auth` 的承载位置是 `cli/auth-command.ts`（导出 `parseAuthCommand`、`validateAuthCommandArgs`、`getAuthCommandUsage`、`printAuthCommandHelp`、`isAuthCommandHelp`）。〔packages/coding-agent/src/cli/auth-command.ts:24-48〕
  - `auth` 由 main 在**主 parseArgs 之前**分派：main 里定义 `runAuthCommand`，并在读取参数解析结果之前调用它（`if (await runAuthCommand(args))`）。〔packages/coding-agent/src/main.ts:132-132〕
  - `config` 的承载位置是 cli/config-selector.ts——文件注释写明它是 `pi config` 用的 TUI 选择器，入口是 `selectConfig`（本卡未展开其参数）。〔packages/coding-agent/src/cli/config-selector.ts:20-20〕
  - 退出码的约定（包管理类）：失败一律 `process.exitCode = 1`（本文件里多处），成功路径是 `process.exit(0)`。〔packages/coding-agent/src/package-manager-cli.ts:861-902〕
  - `auth` 模块自己**不设**退出码：`cli/auth-command.ts` 里只有解析/校验/用法文本与帮助（`parseAuthCommand`、`validateAuthCommandArgs`、`getAuthCommandUsage`、`printAuthCommandHelp`），退出码由 main 的 `runAuthCommand` 决定。〔packages/coding-agent/src/cli/auth-command.ts:24-48〕
- `exportSessionToHtml` 的默认输出路径：未给 `outputPath` 时用 `APP_NAME`-session-`sessionBasename`.html（相对路径，即在 cwd 下），随后 `writeFileSync` 并返回该路径。〔packages/coding-agent/src/core/export-html/index.ts:274-281〕
- 产物是**自包含** HTML：模板 `template.html`/`template.css`/`template.js` 与 vendor 的 `marked.min.js`/`highlight.min.js` 都在生成时读入内联，**不依赖网络**。〔packages/coding-agent/src/core/export-html/index.ts:145-149〕
- 已验证：启动文件里的退出点不止 8 处，但启动阶段（参数与会话解析）的语义清楚，全部先 `console.error` 再 `exit(1)`。〔packages/coding-agent/src/main.ts:305-411〕
  - 参数互斥两组——`--fork` 不能与指定的一组标志同用（`:308-311`）、`--session-id` 不能与另一组同用（`:323-326`）。〔packages/coding-agent/src/main.ts:305-326〕
  - 三个 `…OrExit` 辅助函数——`assertValidSessionId` 抛错（`:328-335`）、`SessionManager.open` 抛错（`:337-345`）、`SessionManager.forkFrom` 抛错（`:347-355`），三处都把 `Error` 的 `message` 或非 Error 值 `String(error)` 包进 `Error: …` 输出。〔packages/coding-agent/src/main.ts:328-355〕
  - 会话定位三处——指定 id 的会话已存在（`:370-373`）、`--session` 未匹配到会话（`:384-387`）、另一处未匹配（`:408-411`），文案分别是 `Session already exists with id '…'` 与 `No session found matching '…'`。除这九处外，模型解析、输出守卫、文件处理器与 experimental 目录下另有若干退出点，属运行期与实验特性，不在启动路径上。〔packages/coding-agent/src/main.ts:305-411〕
- `resolveAppMode` 的判定顺序：`parsed.mode === "rpc"` → rpc；`parsed.mode === "json"` → json；否则若 `parsed.print` 或 `stdinIsTTY`/`stdoutIsTTY` 任一为假 → print；再否则 interactive。〔packages/coding-agent/src/main.ts:111-122〕
- 因此 **rpc 优先级最高**：`stdinIsTTY` 在 rpc 分支之前根本不会被读到，不存在“非 TTY 时另有分支”的差异。〔packages/coding-agent/src/main.ts:111-122〕
- `toPrintOutputMode` 把 json 映射成 json、其余映射成 text。〔packages/coding-agent/src/main.ts:124-126〕
- `@file` 参数在解析阶段就去掉 `@` 存进 `fileArgs`（`result.fileArgs.push(arg.slice(1))`）。〔packages/coding-agent/src/cli/args.ts:236-236〕
- 展开由 `processFileArguments` 做：它累积 `text`（与 `images`）并把两者一起返回。〔packages/coding-agent/src/cli/file-processor.ts:25-30〕
- 调用点在 main 侧：解构 `processFileArguments` 的返回值——也就是**在会话初始消息构造之前**就展开好 `@file`。〔packages/coding-agent/src/main.ts:222-222〕
- `--list-models` 后可跟一个可选值：给了就存字符串，否则存 `true`。〔packages/coding-agent/src/cli/args.ts:206-211〕
- `listModels` 加载 models.json 出错时只在 stderr 打以 `Warning: errors loading models.json:` 开头的黄色警告，**不退出**。〔packages/coding-agent/src/cli/list-models.ts:29-40〕
- 带 search 时用 `fuzzyFilter` 过滤（匹配对象是 provider 与 id 拼出的字符串）；过滤后为空时打印 `No models matching` 一行。〔packages/coding-agent/src/cli/list-models.ts:46-53〕
- 输出是一张表：列含 `provider`、`context`、`max-out`、`thinking`、`images`；每列宽度取表头与该列内容长度的最大值，`context`/`max-out` 经 `formatTokenCount`。〔packages/coding-agent/src/cli/list-models.ts:58-100〕
