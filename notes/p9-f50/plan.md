批次状态：未完成（缺项：Z0 T117 same、Z1 页脚 80 次重绘、Z2 工具输出折行、Z3 输入折行与缩放、Z4 ctrl+p 页脚、Z5 收口）

# F50 清单

起点 HEAD `6aeb4c1`（F49 收口）。

## F49 批次头更正

F49 的批次头应写作 **「未完成（缺项：T117 same）」**：`T117-resume-tool-rows` 三个场景
（含 `-w80`、`L-T117`）在 F49 收口时仍是 `different`，当时以"登记偏离"替代修复，
不符合批次头规则。本批 Z0 修掉后，F49 方可视为完成。

## Z1 测量与归因（改前基线，真实 95 MB 会话 + tmp/realcfg 全扩展）

工具：pty + `PI_TUI_TIMING`（每帧记 `lines/bytes/build/diff/write`），110×40。

| 配置 | 首帧 | 整屏重绘帧数 | 累计写出 |
| --- | --- | --- | --- |
| `-ne`（无扩展） | 4.1 s | **1** | 0.98 MB |
| 全扩展（含 pi-ai-summary） | 4.1 s | **4**（改前） | 3.90 MB |
| 仅 pi-ai-summary | 4.1 s | **80** | 77 MB |
| 仅 pi-handoff-gate | 4.1 s | 1 | 0.98 MB |

**结论（已验证）**：
1. 首帧本身不慢（4.1 s，其中 jsonl 解析 0.62 s + 首帧渲染 ~20 ms + 写出 ~100 ms）。
2. **"反复刷新"的主因是 pi-ai-summary 的页脚接管**：仅装它时出现 **80 次整屏重绘**、
   累计写出 77 MB —— 每次都把整段历史重写一遍，正是用户看到的"从头到尾反复刷新"。
   仅装 pi-handoff-gate 时只有 1 次。
3. ref 侧：`tui-main-screen.ts:455-459` —— `firstChanged < prevViewportTop` 时
   **必须** `fullRender(true)`（全量重写）。所以"整屏重绘"本身是 ref 行为，
   差异在于**触发次数**：TS 的组件更新在一帧内批量完成，我们每来一条扩展 UI 事件就画一帧。
4. 已排除的假设：
   - ~~历史工具行逐条 ext.render.tool 回包导致反复重绘~~：实测把逐条请求改成**一次批量
     RPC + 一条回包事件**后，整屏重绘 4 → **2** 次、写出 3.9 MB → **1.96 MB**
     （有改善，但不是 80 次的主因）。
   - ~~markdown/语法高亮 O(n²)~~：build 16 ms/帧（7703 行），线性。
   - ~~首帧渲染全部历史~~：已在 F49/Y1 修掉（此会话只渲染 5051 行）。

**改后（本批已落地）**：`ext.render.tool` 改为**批量**协议（一次 RPC 处理整批行、
只回一条 `ToolRowsRendered` 事件），并在整批都无人接管时**停用**该会话的工具行请求。
实测全扩展：整屏重绘 **4 → 2**，累计写出 **3.90 MB → 1.96 MB**；`-ne` 仍是 1 帧 0.98 MB，
与改前逐字节一致（本批未改变无扩展路径）。

**未完成**：pi-ai-summary 页脚那条 80 次重绘的循环尚未定位到具体触发点。
已取到的证据：`ext_ui_digest` 在循环期间**恒定不变**（同一 digest 打印 49 次），
但主循环仍在连续帧里重绘 ⇒ 触发点不在 digest 差异，而在"事件 → 置 dirty"的另一条路径
（`[z1ev]` 只在开头出现 FooterOverride / ToolRowsRendered 各一条，随后连续 40+ 次
`[z1draw]` 无任何事件）。下一步应查：`state.status` 是否在页脚回填时被改写、
以及 `ui_render_pending`/`UiRenderDone` 的配对是否在多事件批里失效。

## 未完成项概览（本批到此为止）

| 项 | 状态 | 说明 |
| --- | --- | --- |
| Z0 T117 三场景 same | **未做** | 已定位到差异是 ref 的**逐工具渲染器**（不是启动屏/user 文本）：bash 要 ` $ `、edit 要 `details.diff` 块且不显示结果文本、`report_to_user` 头部不带参数、未知工具要 args JSON 块。pi-rs 目前统一是 ` <name> <summary>` + 文本正文。 |
| Z1 长会话反复刷新 | **部分** | 首帧 4.1 s 已正常（F49）；整屏重绘 4→2、写出 3.9 MB→2.0 MB（批量 RPC）。**pi-ai-summary 页脚的 80 次重绘循环未定位**（证据：digest 恒定、无事件仍连续重绘）。 |
| Z2 工具输出不折行 | **未做** | — |
| Z3 输入区折行与缩放 | **未做** | — |
| Z4 ctrl+p 页脚模型区 | **未做** | — |
| Z5 收口 | **未做** | — |

### Z0 差异明细（已取证，供下一批直接实施）

同一条合成会话（`fixtures/sessions/f49-y2-tools.jsonl`），TS vs pi-rs：

| 工具 | TS（ref） | pi-rs 现状 |
| --- | --- | --- |
| `bash` | ` $ <command>`（`renderers/bash.ts:43-49` 的 `formatShellCall`，prompt=`$`） | ` bash <command>` |
| `edit` | ` edit <path>` + `renderDiff(details.diff)`（`renderers/edit.ts:85/120-124`），**不显示**结果文本 | ` edit <path>` + 文本正文（`已替换 1 处`） |
| `read` | ` read <path>`，折叠态无正文（`renderers/read.ts:120-122`） | 一致 |
| `report_to_user`（无渲染器） | ` report_to_user`，**不加**参数摘要 | ` report_to_user done`（把首个字符串参数拼在头部） |
| 未知工具（无渲染器） | ` <name>` + 换行 + `JSON.stringify(args, null, 2)` 缩进块（`tool-execution.ts:421-432` 的 `formatToolExecution`） | ` <name> {"x":1}`（单行内联） |
| 提示行 | ` ... (N earlier lines, ctrl+o to expand)`（**1 列前导空格**） | `... (N earlier lines, ctrl+o to expand)`（无前导空格） |

另：T75-bash-60 也受同一处影响（` bash` vs ` $`），但它只比最后一个快照（expanded），
所以 regress 判 same —— 属**未被覆盖的既有差异**，与 Z0 同源。
