# 扩展（pi-rs）

本页只写**已实测**的内容：每条后面标了本批（P9-1-E1c）跑的命令与证据文件。
未验证的明确标"未验证"。与上游 pi 0.87.1 的其余差异见 `LIMITATIONS.md`。

## 1. 前置

- 需要本机有 `node`（实测 v24.21.0；更低版本**未验证**）。
- 找不到 `node` 时整个扩展宿主不加载，打印 `pi-rs: 扩展未加载：找不到 node（…）`，其余功能不受影响。
- `PI_NODE=/path/to/node` 可指定解释器。
- `.ts` 扩展需要 jiti；探测顺序见 §3。

## 2. 目录与发现规则（本批修改过，见 `notes/p9-1e1c/discovery-diff.md`）

实现：`crates/pi-ext-host/src/client.rs` 的 `discover_extensions` /
`discover_extensions_in_order`；`crates/pi-cli/src/ext_tools.rs` 的 `start_host`。

- 发现顺序（ref `core/extensions/loader.ts:749-800`）：
  1. 项目级 `<cwd>/.pi/extensions/`
  2. 全局 `<配置目录>/extensions/`（`PI_CODING_AGENT_DIR` 或 `~/.pi/agent`）
  3. 同一个解析后路径只加载一次
- 目录内（ref `:705-744`）：
  - **直接文件** `*.ts` / `*.js`（我们还额外接受 `*.mjs`）；
  - **子目录**入口 `index.ts` → `index.js` → `index.mjs`；
  - 不递归。
- `package.json` 的 `pi.extensions` 清单**支持**（清单里的相对路径，缺失项忽略；ref `loader.ts:667-687`）。
- `-e/--extension <path>` / `--extension=<path>` **支持**（可重复；文件直接加载，目录按
  "清单 → `index.ts`/`index.js`/`index.mjs` → 目录内发现" 展开；ref `cli/args.ts:176-179`、
  `loader.ts:780-795`）。证据：`crates/pi-cli/tests/p9_e3_extension_flag.rs`。
- 排序：按路径排序（ref 用目录读取顺序，见 §6 的差异）。

实测：把 `tools/ext-host/fixtures/ui-ext.mjs` 以 `ui/index.js`（配 `package.json` 的
`{"type":"module"}`）放进场景的 `<agentDir>/extensions/`，启动屏出现 `[Extensions]` + `  ui`，
`--list-tools` 能看到扩展工具。证据：`scenarios-tui/T88-ui-notify.json`、
`golden-tui/T88-ui-notify/run-1/`（TS 录制）。

## 3. `.mjs` / `.js` 与 `.ts`

- `.mjs` / `.js`：Node 原生 ESM。
- `.ts`：需要 jiti，探测顺序（`tools/ext-host/runner.mjs` 的 `jitiCandidates()`）：
  `PI_JITI=` → 扩展目录及其上两级的 `node_modules/jiti` → 已安装 pi 的 `node_modules/jiti`。
- 加载选项与 ref `core/extensions/loader.ts:496-501` 一致（`moduleCache:false` + `virtualModules` + `tryNative:false`）。

实测：`PI_JITI=<某个 jiti 包目录>` 下，`~/.pi/agent/extensions` 的 **9 个 `.ts` 扩展全部加载**
（8 工具 / 4 命令 / 21 事件）。证据：`notes/p9-1e1c/s3-raw.json`（本地记录，不入库）。

## 4. 如何确认已加载

- 启动屏 `[Extensions]` 段：`  <名字>`（逗号+空格连接，按名字排序；ref `interactive-mode.ts:1821-1832`）。
- `pi-rs --list-tools`：扩展工具 `source=extension`。
- 失败文案：无 node / 找不到 runner / 握手失败 / 协议版本不符，各有对应 stderr 提示。

## 5. 当前支持与不支持

| 能力 | 状态 | 证据 |
| --- | --- | --- |
| `registerTool` | 支持（含两轮模型调用往返） | `crates/pi-cli/tests/p9_1e1c_e2e.rs` |
| 事件 | `session_start`（`startup` / `resume`）、`agent_end`、`tool_call`、`tool_result` | `crates/pi-cli/tests/p9_1e1c_dispatch.rs`、`crates/pi-ext-host/tests/hooks.rs` |
| `input`（`continue` / `transform` / `handled`） | 支持：`transform` 改写的文本真的发给模型；`handled` 不跑 agent、不发请求 | `crates/pi-cli/tests/p9_e2_input.rs`（本地证据 `notes/p9-e2-input.txt`） |
| `before_agent_start` | 支持：返回的 `message` 以 `role:"custom"` 插在用户消息之前、`systemPrompt` 覆盖生效 | `crates/pi-cli/tests/p9_e3b_inject.rs` |
| `agent_before_settle` | 支持 `continue:true` 再跑一轮（上限 10 轮保护）与 `entries`（以 `role:"custom"` 消息落盘并进入下一轮） | `crates/pi-cli/tests/p9_e3b_settle.rs`、`p9_e3b_settle_entries.rs` |
| `tool_call` 的 `block`/`reason` | 支持 | `crates/pi-cli/tests/p9_1d_e2e.rs` |
| `tool_result` 的替换 | 支持（链式；落盘与下一轮请求体都是替换后内容） | `crates/pi-cli/tests/p9_1e1c_e2e.rs` |
| `ctx`：`cwd`/`mode`/`hasUI`/`model`/`thinkingLevel` | 支持（`model`/`thinkingLevel` 随切换更新） | `crates/pi-cli/tests/p9_1e1_ctx.rs` |
| `ctx.ui.notify(message, type)` | 支持：`info` → 会话区状态行；`warning`/`error` → `Warning: ` / `Error: ` 行 | `scenarios-tui/T88-ui-notify.json`（TS 逐帧 same） |
| `ctx.ui.setStatus(key, text)` | 支持：footer 下方单独一行，按 key 排序、空格连接、清洗+截断；`undefined` 清除 | 同上 |
| `ctx.ui.select/confirm/input`（**无 UI**：print/rpc） | 支持：按 ref `runner.ts:320-325` 的 `noOpUIContext` 返回 `undefined`/`false` | `crates/pi-cli/tests/p9_e2_ui_noui.rs` |
| `ctx.ui.select`（**TUI**） | 支持：面板版式与按键照 ref `extension-selector.ts`，与 TS 录制逐帧 `same` | `scenarios-tui/T89-ext-select.json`、`scenarios-tui-light/L-T89-ext-select.json` |
| `ctx.ui.confirm/input`（**TUI**） | 支持（同一套面板），**未单独录制 golden** | `crates/pi-ext-host/tests/ui_ask.rs`（宿主往返） |
| `ctx.ui.setWidget(key, string[], {placement})` | 支持 `string[]` 形态（`aboveEditor`/`belowEditor`，按 key 升序、逐行渲染）；`undefined` 清除 | `pi-tui` 单测 `extension_widget_tests` |
| 宿主动作帧（`createBashToolDefinition` 的 `bash`） | 支持：执行经 `host.action.call` 交回宿主（`pi_core` 的 bash 工具） | `crates/pi-ext-host/tests/action_call.rs`、`tests/matrix.rs` |
| `ctx.sessionManager.getSessionFile()/getEntries()` | 支持（只读）：宿主给会话文件路径，扩展按需读文件解析条目 | `crates/pi-ext-host/tests/session_manager.rs` |
| `pi.getSessionName()` | 返回 `undefined`（pi-rs 未实现会话命名，ref `types.ts:1509`） | `crates/pi-ext-host/tests/session_name.rs` |
| `pi.sendUserMessage(text)` | 支持（print 路径：排队的消息在下一轮注入；上限与 settle 共用） | `crates/pi-cli/tests/p9_e3b_sendmsg.rs` |
| `pi.appendEntry(customType, data)` | 支持（写 `{type:"custom", customType, data}` 到会话文件） | `crates/pi-cli/tests/p9_e3b_append.rs` |
| `ctx.getContextUsage()` | 支持：宿主在 ctx 里带 `usage{tokens,contextWindow,percent}`，扩展读到同一份 | `crates/pi-ext-host/tests/ctx_usage.rs` |
| `setWidget`（组件工厂）、`setHeader`/`setFooter` | **不支持** | — |
| `registerCommand` | 支持：`/name args` 命中扩展命令 ⇒ 调 handler、不跑 agent；未命中照常当 prompt；handler 抛错按 ref 视为"已处理" | `crates/pi-cli/tests/p9_e3_commands.rs` |
| `registerFlag` / `getFlag` | 支持（两阶段解析）：`--demo-flag=v` / `--switch` 经扩展注册表校验后由 `pi.getFlag` 读取；未注册报 `Unknown option(s)` | `crates/pi-cli/tests/p9_e3b_flags.rs` |
| `renderCall`（`Text`/`Spacer`/`Container`） | 支持：工具块用扩展给的降级行替代默认摘要行；TS 录制 dark/light 逐帧 same（`T90-render-hook`） | `scenarios-tui/T90-render-hook.json` |
| `renderResult`（`Text`/`Spacer`/`Container`） | 支持：结果区用扩展的降级行替代内置正文；TS 录制 dark/light 逐帧 same，稳定性各 10/10 | `scenarios-tui/T90-render-hook.json`、`notes/p9-1e4b/s4-stability.txt` |
| 其它组件（`Box`/`Markdown`/`Table`/`SelectList`） | 不支持：整块回落内置渲染（`unsupported`） | `tests/p9_1e4a_frames.rs` |

## 6. 与 ref 的差异（逐条，均有本批或前批证据）

- **overlay**：`ctx.ui.custom(..., { overlay: true })` **支持**——由 `pi-cli` 在 frame 之上合成
  （垂直居中、盒宽 80 列、盒内清空写文字、盒外保留）；TS 录制 `T95-overlay`/`L-T95-overlay`
  dark/light 逐帧 same、各 10 次稳定。`overlayOptions` 的 anchor/具体尺寸**未实现**、
  `onHandle` 仅提示未实现。
- **并发**：工具调用串行；UI 按键/渲染钩子可在 invoke 期间并发（`Wire` 直连，`in_flight>0` 时）。
- **发现规则**：支持 `-e/--extension`、`package.json` 的 `pi.extensions`、`index.ts/js/mjs`；
  未实现 ref 的"目录读取顺序"（我们按路径排序）。
- **flag**：两阶段解析已实现，`--help` 不列扩展 flag（与 ref 同）。

## 7. 与 ref 的差异（历史批次的原始记录）

- 发现：无 `package.json` 清单、无 `-e`；排序按路径而 ref 用目录读取顺序。
- `*.mjs` 入口是我们额外接受的（ref 只认 `.ts`/`.js`）。
- `ctx.ui.setStatus` 的状态行内容与 ref 一致，但**颜色/样式未逐字节对拍**。
- `notify` 的空行位置：我们复用 `Notice` 项自带的行尾空行，ref 是显式 `Spacer(1)`；
  文本逐帧一致（T88 both themes `same`），空行计数未单独验证。

## `user_bash`（未实现）

`pi-secrets` 等扩展订阅 `user_bash` 以对用户 `!` 命令做脱敏。pi-rs 目前**没有用户态 bash 入口**
（`!`/`!!` 未实现），因此该事件不会派发。ref 语义：`interactive-mode.ts:6725-6745`（事件形状
`{type:"user_bash", command, excludeFromContext, cwd}`，扩展可返回 `{result}` 直接采用或
`{operations}` 替换执行；扩展抛错时不回退本地执行），落盘形态
`agent-session.ts:3496-3516`（`role:"bashExecution"`）。
