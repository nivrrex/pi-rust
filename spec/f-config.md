# spec/f-config.md — settings.json / models.json / auth.json

范围限定为本项目要互通与要实现的部分：字段定义、路径解析、多作用域合并、环境变量。
TUI 外观类设置（`terminal`、`theme`、`markdown`、`warnings`、全屏相关）只记字段名与默认值来源，不逐项展开。

## 1. 配置目录与环境变量

- `CONFIG_DIR_NAME` 取自包清单的 `piConfig.configDir`，缺省 `".pi"`。〔packages/coding-agent/src/config.ts:504-504〕
- 环境变量名由应用名拼出来：`ENV_AGENT_DIR = ${APP_NAME.toUpperCase()}_CODING_AGENT_DIR`，`ENV_SESSION_DIR = ${APP_NAME.toUpperCase()}_CODING_AGENT_SESSION_DIR`；注释举例 `PI_CODING_AGENT_DIR`。〔packages/coding-agent/src/config.ts:507-509〕
- `getAgentDir()` 优先读 `process.env[ENV_AGENT_DIR]`（经 `expandTildePath` 处理 `~`），否则 `join(homedir(), CONFIG_DIR_NAME, "agent")`，即默认 `~/.pi/agent`。〔packages/coding-agent/src/config.ts:511-513〕〔packages/coding-agent/src/config.ts:527-534〕
- `PI_SHARE_VIEWER_URL` 覆盖 `/share` 的基础 URL，默认 `https://pi.dev/session/`。〔packages/coding-agent/src/config.ts:515-521〕
- 离线开关的环境变量是 `PI_OFFLINE`，判定入口是 `isOfflineModeEnabled()` 读 `process.env.PI_OFFLINE`；`--offline` 只是把它归一化后再影响启动期的模型目录刷新与远程解析，它**不拦截 provider 请求**。**待验证**〔packages/coding-agent/src/core/package-manager.ts:53-57〕

## 2. settings.json 的两个作用域与合并

- 路径：全局是 `<agentDir>/settings.json`，项目是 `<cwd>/.pi/settings.json`（用 `CONFIG_DIR_NAME` 拼）。〔packages/coding-agent/src/core/settings-manager.ts:231-238〕
- 合并入口：`this.settings = deepMergeSettings(this.globalSettings, this.projectSettings)`，即**项目覆盖全局**。〔packages/coding-agent/src/core/settings-manager.ts:343-350〕
- `deepMergeObjects` 的规则：以 base 为起点遍历 overrides 的键，`undefined` 值直接跳过（因此无法用 `undefined` 覆盖）；base 与 override 都是可合并对象（`typeof === "object"`、非 null、非数组）时递归合并，否则整体取 override。〔packages/coding-agent/src/core/settings-manager.ts:165-186〕
- 因此数组是**整体替换**而不是合并：`isMergeableObject` 明确排除 `Array.isArray(value)` 的值。〔packages/coding-agent/src/core/settings-manager.ts:165-167〕
- 加载错误分开记录：`globalSettingsLoadError` 与 `projectSettingsLoadError` 两个独立字段，供诊断而不影响另一侧。〔packages/coding-agent/src/core/settings-manager.ts:318-327〕
- 作用域类型只有两个值 `"global"` 与 `"project"`（`SettingsScope`），写盘按 `scope === "global"` 选路径。〔packages/coding-agent/src/core/settings-manager.ts:204-204〕〔packages/coding-agent/src/core/settings-manager.ts:269-269〕
- `getGlobalSettings()` / `getProjectSettings()` 返回 `structuredClone`，避免调用方改到内部状态。〔packages/coding-agent/src/core/settings-manager.ts:503-507〕

## 3. settings.json 在本项目范围内的字段

- 模型与推理：`defaultProvider`、`defaultModel`、`defaultThinkingLevel`、`modelThinkingLevels`（键是 `"provider/modelId"`）、`thinkingBudgets`。〔packages/coding-agent/src/core/settings-manager.ts:112-115〕〔packages/coding-agent/src/core/settings-manager.ts:147-147〕
- 队列：`steeringMode` 与 `followUpMode`，取值都是 `"all" | "one-at-a-time"`。〔packages/coding-agent/src/core/settings-manager.ts:117-118〕
- 压缩：`compaction`（含 `enabled`/`reserveTokens`/`keepRecentTokens`/`modelOverrides`，见 `spec/d-compaction.md` 第 2 节）与 `branchSummary`。〔packages/coding-agent/src/core/settings-manager.ts:120-121〕
- 重试：`retry`，与 `RetryPolicy` 同字段（`enabled`/`maxRetries`/`baseDelayMs`/`maxAgentDelayMs`）。〔packages/coding-agent/src/core/settings-manager.ts:122-122〕〔packages/ai/src/utils/retry.ts:94-109〕
- 工具：`defaultTools`（内置工具初始选择）、`packages`/`extensions`/`skills`/`prompts`/`themes`（本地路径或包源数组）、`enableSkillCommands`。〔packages/coding-agent/src/core/settings-manager.ts:135-144〕
- bash 执行：`shellPath`、`shellCommandPrefix`、`exposeSessionEnvironment` 相关的默认值（后者是工具选项而非 settings 字段）。〔packages/coding-agent/src/core/settings-manager.ts:126-129〕
- 会话：`sessionDir`（与 `--session-dir` 同格式）。〔packages/coding-agent/src/core/settings-manager.ts:154-154〕
- 网络：`httpProxy`（同时作为 `HTTP_PROXY` 与 `HTTPS_PROXY`）、`httpIdleTimeoutMs`（0 表示禁用）、`websocketConnectTimeoutMs`、`transport`（默认 `"auto"`）。〔packages/coding-agent/src/core/settings-manager.ts:116-116〕〔packages/coding-agent/src/core/settings-manager.ts:155-158〕
- 缓存预热：`cacheWarming`（默认 `"streaming"`，注释标明是全局设置，因为每次刷新都要花钱）。〔packages/coding-agent/src/core/settings-manager.ts:157-157〕
- 会话树/导航：`doubleEscapeAction`（`"fork" | "tree" | "none"`，默认 `"tree"`）、`treeFilterMode`。〔packages/coding-agent/src/core/settings-manager.ts:145-146〕
- TUI 相关（本项目只记字段名）：`theme`、`quietStartup`、`terminal`、`images`、`hideThinkingBlock`、`showCacheMissNotices`、`editorPaddingX`、`outputPad`、`autocompleteMaxVisible`、`showHardwareCursor`、`markdown`、`warnings`、`externalEditor`、`tuiMode`、`fullscreenExitOutput`、`fullscreenScrollbar`、`fullscreenCopyOnSelect`。〔packages/coding-agent/src/core/settings-manager.ts:119-163〕
- 项目信任：`defaultProjectTrust`（默认 `"ask"`），注释标明**仅全局设置生效**。〔packages/coding-agent/src/core/settings-manager.ts:128-128〕
- 遥测：`enableInstallTelemetry`（默认 true）、`enableAnalytics`（默认 false，需 opt-in）、`trackingId`、`lastChangelogVersion`、`collapseChangelog`。〔packages/coding-agent/src/core/settings-manager.ts:111-111〕〔packages/coding-agent/src/core/settings-manager.ts:131-134〕
- 包管理命令：`npmCommand` 是 argv 风格数组（注释举例 `["mise", "exec", "node@20", "--", "npm"]`）。〔packages/coding-agent/src/core/settings-manager.ts:130-130〕
- 超时类字段的解析失败会抛 `Invalid ${settingName} setting: ${String(value)}`，即无效值不是静默忽略。〔packages/coding-agent/src/core/settings-manager.ts:193-202〕

## 4. models.json

- 文件位置由 `join(agentDir, "models.json")` 得出。〔packages/coding-agent/src/core/agent-session-services.ts:144-144〕
- 顶层只有一个必需字段 `providers`，是 `Record<string, ProviderConfigSchema>`。〔packages/coding-agent/src/core/model-config.ts:242-244〕
- `ProviderConfigSchema` 字段：`name?`、`baseUrl?`、`apiKey?`、`api?`、`oauth?`（只允许字面量 `"radius"`）、`headers?`、`compat?`、`authHeader?`、`models?`、`modelOverrides?`；其中 `name`/`baseUrl`/`apiKey`/`api` 都带 `minLength: 1`。〔packages/coding-agent/src/core/model-config.ts:229-240〕
- `models` 是 `ModelDefinitionSchema[]`，`modelOverrides` 是 `Record<string, ModelOverrideSchema>`。〔packages/coding-agent/src/core/model-config.ts:238-239〕
- 解析前先 `stripJsonComments(stripBom(content))`，即 **models.json 允许注释**。〔packages/coding-agent/src/core/model-config.ts:295-303〕
- 校验用 typebox `Compile(ModelsConfigSchema)`；不通过时错误文本是 `Invalid models.json schema:` 后跟每行 `  - <路径>: <消息>`，路径由 instancePath 把 `/` 换成 `.` 得到。〔packages/coding-agent/src/core/model-config.ts:252-263〕〔packages/coding-agent/src/core/model-config.ts:305-312〕
- 文件不存在（`ENOENT`）不算错误，返回空 provider 表；其他读失败返回带 `Failed to load models.json: ...` 的空配置。〔packages/coding-agent/src/core/model-config.ts:285-293〕
- JSON 解析失败返回带 `Failed to parse models.json:` 前缀的空配置（读文件失败则是 `Failed to load models.json:`）。〔packages/coding-agent/src/core/model-config.ts:285-303〕
- 加载成功后每个 provider 都做 `deepFreeze(structuredClone(provider))`，即快照不可变且与源对象无引用关系。〔packages/coding-agent/src/core/model-config.ts:265-269〕〔packages/coding-agent/src/core/model-config.ts:314-319〕
- 路径参数为 `undefined` 时直接返回空 `ModelConfig`（不读文件）。〔packages/coding-agent/src/core/model-config.ts:281-282〕

## 5. auth.json

- 默认路径 `<agentDir>/auth.json`，构造时 `normalizePath` 规范化。〔packages/coding-agent/src/core/auth-storage.ts:49-54〕
- 父目录不存在时 `mkdirSync(dir, { recursive: true, mode: 0o700 })`。〔packages/coding-agent/src/core/auth-storage.ts:56-60〕
- 写文件的选项是 `{ encoding: "utf-8", mode: 0o600 }`，注释说明 mode 只在创建时生效，以免破坏管理员管理的模式与 ACL。〔packages/coding-agent/src/core/auth-storage.ts:24-25〕
- 文件内容形状是"每个 provider 一个带 type 标签的凭据"：`Credential = ApiKeyCredential | OAuthCredential`。〔packages/ai/src/auth/types.ts:36-37〕
- `ApiKeyCredential` 是 `{ type: "api_key"; key?: string; env?: ProviderEnv }`，注释说明 `env` 放 provider 作用域的环境/配置值（例如 Cloudflare 的 account/gateway id）。〔packages/ai/src/auth/types.ts:13-21〕
- `OAuthCredential` 继承 `OAuthCredentials`（`refresh`、`access`、`expires`，外加 `[key: string]: unknown`）并加 `type: "oauth"`。〔packages/ai/src/auth/types.ts:23-34〕
- 读取时校验：顶层必须是对象，否则抛 `Invalid auth.json: expected an object`；单个 provider 的值不合法抛 `Invalid auth.json credential for provider "${providerId}"`。〔packages/coding-agent/src/core/auth-storage.ts:222-230〕〔packages/coding-agent/src/core/auth-storage.ts:251-251〕
- 取 API key 时要求 `credential.type === "api_key"` 且 `key` 非空，且 `key` 不能是命令配置值（`isCommandConfigValue`）。〔packages/coding-agent/src/core/auth-storage.ts:263-263〕
- 只读模式下任何修改都抛 `Read-only credential storage cannot modify auth.json`。〔packages/coding-agent/src/core/auth-storage.ts:284-288〕
- 凭据存储以 `modify` 作为**唯一**写路径，因此每次变更都是串行的读-改-写；OAuth 刷新也在 `modify` 内进行，避免并发请求重复刷新已轮换的 token。读操作对缺失项 resolve `undefined`，只在存储失败时 reject。〔packages/ai/src/auth/types.ts:52-60〕
- 运行时可用 `--api-key` 覆盖：`override ? { type: "api_key", key: override } : this.store.read(providerId, options)`。〔packages/coding-agent/src/core/runtime-credentials.ts:27-27〕
- 本项目对 OAuth 的范围是**只读**（PLAN 第 0 节的排除清单写明只读 auth.json 中的 API key、遇到 OAuth 凭据给出清晰错误）；ref 侧的凭据形态是二选一联合：`Credential` = `ApiKeyCredential` | `OAuthCredential`。〔packages/ai/src/auth/types.ts:24-37〕
- `OAuthCredential` 是 `OAuthCredentials` 的扩展，即 OAuth 之外主要是 `ApiKeyCredential`。〔packages/ai/src/auth/types.ts:32-37〕

## 5b. skills 与 prompt templates 的发现

- 已验证：全局配置目录下的技能与提示词模板会被发现并注册为斜杠命令——`get_commands` 的返回里同时出现 `skill:review-helper` 与 `change-summary`（`golden/S24b/events.jsonl` 的同名响应亦可复现）。〔golden/S24b/events.jsonl:1-1〕
- 已验证：斜杠命令在输入时会被展开，命令名与 `get_commands` 列出的完全一致：用 `/skill:review-helper <参数>` 调用后，请求体里不再有字面斜杠文本，skill 正文（`请按要点逐条检查改动`）进入对话。〔golden/S24/requests.jsonl:1-1〕
- 已验证：展开在 print 模式（命令行消息）与 rpc 的 `prompt` 通道都发生，后者同样不含字面斜杠文本、含 skill 正文。用错名字（`/review-helper` 而不是 `/skill:review-helper`）时不会展开，会被原样送进请求体。〔golden/S24b/requests.jsonl:1-1〕

## 6. 多作用域小结

- 只有 `settings.json` 是双作用域（全局 + 项目）；models.json 与 auth.json 都在 agentDir 下各一份。〔packages/coding-agent/src/core/agent-session-services.ts:144-144〕〔packages/coding-agent/src/core/auth-storage.ts:49-54〕〔packages/coding-agent/src/core/settings-manager.ts:231-238〕
- 项目 settings 的路径由**进程 cwd** 决定：`this.projectSettingsPath = join(resolvedCwd, …)`，其中 `CONFIG_DIR_NAME` 从 `core/paths` 导入。〔packages/coding-agent/src/core/settings-manager.ts:238-240〕
- 按 scope 取路径的写法同样用 `resolvedCwd`：`project: join(resolvedCwd, CONFIG_DIR_NAME, …)`——也就是 settings 看的是 cwd，而会话自己的 cwd 记在会话文件 header 里（本项目实测：header 的 `cwd` 与运行目录不一致时，该会话不会被当成当前会话）。〔packages/coding-agent/src/core/settings-manager.ts:360-368〕

## 未覆盖 / 不确定

- `ModelDefinitionSchema` 的字段：`id`（**必填**，minLength 1）与可选的 `name`/`api`/`baseUrl`/`reasoning`/`thinkingLevelMap`/`input`/`inputLimits`/`cost`/`promptCache`/`contextWindow`/`maxTokens`/`samplingParams`/`headers`/`compat`。〔packages/coding-agent/src/core/model-config.ts:188-204〕
- `ModelOverrideSchema` 是**全可选**的补丁：`name`/`reasoning`/`thinkingLevelMap`/`input`/`inputLimits`/`cost`（内联 `input`/`output`/`cacheRead`/`cacheWrite`/`tiers`）/`promptCache`/`contextWindow`/`maxTokens`/`samplingParams`/`headers`。〔packages/coding-agent/src/core/model-config.ts:206-240〕
- `input` 的取值是 `text` | `image` 的字面量数组；`samplingParams` 是 string→unknown、`headers` 是 string→string 的 record。〔packages/coding-agent/src/core/model-config.ts:195-203〕
- 已验证：`compat` 是三个对象 schema 的联合，按模型所属 provider 取对应分支；各分支字段全部可选。〔packages/coding-agent/src/core/model-config.ts:182-203〕
  - `OpenAICompletionsCompatSchema`（`:73-112`）：`supportsStore`、`supportsDeveloperRole`、`supportsReasoningEffort`、`supportsUsageInStreaming`、`supportsFinishReason`、`maxTokensField`（`max_completion_tokens` 等字面量联合）、`requiresToolResultName`、`requiresAssistantAfterToolResult`、`requiresThinkingAsText`、`requiresReasoningContentOnAssistantMessages`、`thinkingFormat`、`chatTemplateKwargs`、`chatTemplateArgs`、`cacheControlFormat`（固定 `"anthropic"`）、`openRouterRouting`、`vercelGatewayRouting`、`supportsOpenAIGrammarTools`、`supportsStrictMode`、`sendSessionAffinityHeaders`、`sessionAffinityFormat`（`openai` / `openai-nosession` / `openrouter`）、`supportsLongCacheRetention`、`vllmPriority`。〔packages/coding-agent/src/core/model-config.ts:73-112〕
  - `OpenAIResponsesCompatSchema`（`:114-122`）：`supportsDeveloperRole`、`sessionAffinityFormat`、`supportsLongCacheRetention`、`supportsStrictMode`、`supportsOpenAIGrammarTools`、`supportsMaxOutputTokens`。〔packages/coding-agent/src/core/model-config.ts:114-122〕
  - `AnthropicMessagesCompatSchema`（`:160-180`）：`supportsEagerToolInputStreaming`、`supportsLongCacheRetention`、`sendSessionAffinityHeaders`、`supportsCacheControlOnTools`、`supportsTemperature`、`forceAdaptiveThinking`、`allowEmptySignature`、`supportsStrictTools`、`supportsMidConvoEffort`、`allowedFallbackModels`（数组，元素含 `provider`/`model`/`cost`，**最多 3 个**）。〔packages/coding-agent/src/core/model-config.ts:160-180〕
- settings 里的字段是 `branchSummary?: BranchSummarySettings`。〔packages/coding-agent/src/core/settings-manager.ts:121-121〕
- 取值规则：`reserveTokens: this.settings.branchSummary?.reserveTokens ?? 16384`、`skipPrompt: this.settings.branchSummary?.skipPrompt ?? false`（缺省 16384 / false）。〔packages/coding-agent/src/core/settings-manager.ts:910-911〕
- 它在 `navigateTree` 流程里被读成 `branchSummarySettings`，并把 `reserveTokens` 传给分支摘要。〔packages/coding-agent/src/core/agent-session.ts:3677-3686〕
- `withLock(scope, fn)`：按 `scope` 选 `globalSettingsPath` 或 `projectSettingsPath`；文件存在或需要写时才拿锁，把当前内容交给 `fn`，`fn` 返回非 `undefined` 时补建目录、`writeFileSync(path, next, "utf-8")`、然后在 `finally` 里释放锁。〔packages/coding-agent/src/core/settings-manager.ts:268-292〕
- 锁用外部库：`lockfile.lockSync(path, { realpath: false })`，外层带重试。〔packages/coding-agent/src/core/settings-manager.ts:240-250〕
- 写回是**直接** `writeFileSync`，没有“临时文件 + rename”→ **不是原子写**。〔packages/coding-agent/src/core/settings-manager.ts:285-292〕
- `enabledModels` 里的模式串形如 `provider/id`（由 `modelReference` 拼出），匹配**不区分大小写**；未命中时把该串追加进列表。〔packages/coding-agent/src/core/agent-session.ts:2146-2151〕
- `getEnabledModels()` 读 `this.settings.enabledModels`；`setEnabledModels()` 写的是**全局** settings，并 `markModified` + `save`。〔packages/coding-agent/src/core/settings-manager.ts:1333-1345〕
- `defaultTools` 在 settings 类型里是 `string[]`（注释说明它是最初的内置工具集）。〔packages/coding-agent/src/core/settings-manager.ts:144-144〕
- 上下文文件（AGENTS.md 一类）的候选名顺序：`AGENTS.override.md` → `AGENTS.md` → `AGENTS.MD` → `CLAUDE.md` → `CLAUDE.MD`。〔packages/coding-agent/src/core/resource-loader.ts:72-72〕
- 加载顺序是**先全局后项目**：先从 `resolvedAgentDir` 取，再按 `currentDir` 逐级找；已加载的路径记进 `seenPaths` 去重。〔packages/coding-agent/src/core/resource-loader.ts:129-142〕
- skills 由 `loadSkills` 载入并挂在 `skills` 字段上，经 `getSkills()` 暴露；另有 `skillsOverride` 钩子允许替换整批结果。〔packages/coding-agent/src/core/resource-loader.ts:26-42〕
- **更正（本轮推翻此前结论）**：先前写的「pi 不读任何 API key 环境变量」是错的——那次检索只覆盖了 coding-agent 一个包，漏了 ai 包下的 provider 目录，那里用 `envApiKeyAuth` 读环境变量。凭据来源实为四类，详见下面几条。〔packages/ai/src/providers/openai.ts:11〕
  - **provider 层确实读环境变量**：provider 文件里的 `envApiKeyAuth` 会读环境变量里的 key，anthropic 的解析顺序见下条。〔packages/ai/src/providers/openai.ts:11〕
  - **凭据解析顺序**（以 anthropic 为例）：已存凭据 → `ANTHROPIC_AUTH_TOKEN` → 依次试 `ANTHROPIC_OAUTH_TOKEN`、`ANTHROPIC_API_KEY`。〔packages/ai/src/providers/anthropic.ts:19-39〕
  - **交给 SDK 的形状只有两种**：`apiKey` 或 `headers`；具体用哪个头由 SDK 决定，pi 自己不硬编码——校验函数只查有没有，不查是哪个。〔packages/ai/src/providers/anthropic.ts:19-39〕
  - 原先仍然成立的部分：凭据也可来自 models.json 的 `provider.apiKey`、CLI 的 `--api-key`、`oauth` 字段；`PI_PROVIDER`/`PI_MODEL` 只在 evals 里，`PI_SESSION_ID`/`PI_REASONING_LEVEL` 只在测试里。〔packages/coding-agent/src/core/model-config.ts:228-239〕
  - **具体头名由谁发出，未定位**：能确认 anthropic 的 auth token 分支把凭据放进 `headers`（`Authorization` 加 `Bearer` 前缀）；而 `apiKey` 形态最终变成什么头由 SDK 负责，本轮没追到 SDK 调用处。〔packages/ai/src/providers/anthropic.ts:19-39〕


#### agent 侧读取的 PI_* 环境变量

按代码中出现次数排序：

```
PI_OFFLINE  PI_CODING_AGENT_DIR  PI_PACKAGE_DIR  PI_EXPERIMENTAL  PI_SKIP_VERSION_CHECK
PI_CACHE_RETENTION  PI_TELEMETRY  PI_NO_LOCAL_LLM  PI_EVAL_VARIANT  PI_EVAL_CONTAINER
PI_TIMING  PI_STARTUP_BENCHMARK  PI_TUI_DEBUG  PI_TUI_DEBUG_REDRAW  PI_TUI_WRITE_LOG
PI_TRUE_COLOR  PI_TEST_NATIVE_CLIPBOARD  PI_SHARE_VIEWER_URL  PI_CLEAR_ON_SHRINK
PI_INSTALLER_API_BASE  PI_MANAGED_INSTALL_ROOT  PI_HARDWARE_CURSOR
```
  - `PI_PROVIDER` / `PI_MODEL` 只出现在 `packages/evals/`（评测 harness 的 CLI 与用例），**不是 pi 的运行时接口**；`PI_SESSION_ID` / `PI_REASONING_LEVEL` 只在 `packages/coding-agent/test/` 的测试里作为被透传给扩展的环境出现，运行时同样不读。原条目把它们与 `*_API_KEY` 并列是错的，已更正。〔packages/evals/src/cli.ts:74-76〕
- `isOfflineModeEnabled()` 在 `packages/coding-agent/src` 里有**两份完全相同**的实现（`utils/tools-manager.ts:14-18` 与 `core/package-manager.ts:53-57`），**不在** `packages/ai` 下。〔packages/coding-agent/src/core/package-manager.ts:53-57〕
- 判定语义：`process.env.PI_OFFLINE` 为空/未设时为 false；否则仅在值为 `1`、或（转小写后）`true`/`yes` 时为 true。〔packages/coding-agent/src/utils/tools-manager.ts:14-18〕
