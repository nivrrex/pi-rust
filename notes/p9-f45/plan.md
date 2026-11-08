# F45 清单

起点 HEAD `65ff1ef`（F44 收口）。

## T1 段落直接接列表
- ref 规则（`packages/tui/src/components/markdown.ts:487-497`）：
  `case "paragraph"` 后 `if (nextTokenType && nextTokenType !== "list" && nextTokenType !== "space") lines.push("")`
  ⇒ **段落后面紧跟列表或空行时不补空行**；我们的 `Start(List)` 里已有同样判断（`lib.rs:4159-4180`）。
- 新夹具 `tools/difftest/md-corpus/list-after-para.md`（段落＋有序列表、段落＋无序列表、
  列表后紧跟段落；列表项含 `**粗体**`、行内代码、长 CJK 折行）。
- `md-diff --ignore-blank`：110 列 `different=0`。80/100 列的 2 处差异经逐行核对
  **不是空行**，而是折行断点（见 T2） ⇒ T1 本身无缺陷，空行逻辑与 ref 一致。

## T2 CJK 折行溢出（部分修复，登记残余）
取证（逐宽度 78/79/80/81/82/100/110 + 逐行 visibleWidth）：
- ref `utils.ts:876-962` 的 `wrapSingleLine` **按 token** 折行，token 由
  `splitIntoTokensWithAnsi` 切分：CJK 字符（`cjkBreakRegex` = Han/Hiragana/Katakana/Hangul/Bopomofo，
  `:789-812`）**各自成 token**；连续空格成 space token；其余合成不可分 word token；
  判定是 `currentVisibleLength + tokenVisibleLength > width`（**恰好等于允许**）。
- 我们的 `wrap_to_width` 原来**按字符**判定，且把 `0x3000-0x303F`（CJK 符号标点）与
  `0xFF00-0xFFEF`（全角）也算作 CJK 断点 —— 这两段**不在** ref 的 `cjkBreakRegex` 里
  （`（）；：` 等 Script 是 Common）⇒ 断点位置整体偏移 1–2 格（真机"多放 4-6 格"的来源）。
- 修法：`wrap_to_width` 改为 token 级（`tokens_for_wrap`），`is_cjk_breakable` 只保留
  ref 的五个脚本区间。
- 可用宽最终公式：`cols - 2 + lead_all.len()`（`cols-2` 是 `Markdown(paddingX=1)` 的内容区，
  行首缩进要加回；列表 marker 已含在行文本里、与 ref 的 `itemWidth` 相抵）。
- 结果：`md-diff --ignore-blank` 在 110/100/80 三档下 `list-hang`、`list-after-para`、
  `table` **全部 different=0**；`basic` 剩 1 处、`nested` 剩 10 处均为**既有样式类差异**
  （SGR 分段方式 / 引用块颜色），与折行无关。

## T3 恢复会话的模型与占比（已修，两处同源）
- ref 取证：
  - 上下文占比：`modes/interactive/components/footer.ts:110-113` 取 `session.getContextUsage()?.percent`
    与 `?.contextWindow`；`core/agent-session.ts:3858-3863` 的 `getContextUsage()` **先取 `this.model`，
    没有模型直接 `return undefined`**（占比段退化成 `?`），窗口取 `model.contextWindow`。
  - 恢复模型：`core/session-manager.ts:418-434` 的 `getSessionContextSettings()` —— 沿分支取
    **最后一个** `model_change` 或最后一条 assistant 消息的 `message.provider/model`。
- pi-rs 根因：全仓库只在**写入**时产生 `model_change`（`session_writer.rs`、`tui.rs`、`rpc_session.rs`），
  **恢复路径从不解析它** ⇒ 模型停留在 settings 默认值（真机 `no-model`），
  `getContextUsage` 等价路径拿不到 `contextWindow` ⇒ 占比 0.0%。**两者同源，证据即同一处缺失**。
- 夹具 `scenarios-tui/T108-resume-model.json`：会话里的 `model_change`/assistant 用 `mw-auto/auto`
  （与 settings 默认的 `openai-completions/mock-model` **不同**）⇒ 可以区分是否读了会话。
- 改前：pi-rs 底栏 `… (openai-completions) mock-model`（用 settings）；TS `… auto`。
  改后：两侧一致 `(mw-auto) auto`。golden 入库（`same`）。
- 回退行为：会话里**没有** `model_change` 且无 assistant 消息时，`resumed_model` 为 `None` ⇒
  回落到 `resolve_startup_model`（settings / 命令行），与 ref 的 `model: null` ⇒ 上层回落一致；
  ref 在会话无模型时 `getContextUsage()` 返回 undefined ⇒ 占比显示 `?`，pi-rs 走
  「无 usage 时按系统提示估算」，属已登记的既有差异（`LIMITATIONS`）。

## T4 Todo 部件颜色（已修）
- 取证（color-dump，dark，100 列）：TS `0-0:-/- 1-10:i143/-`，pi-rs `1-10:rgb(181,189,104)/-`。
  两者视觉接近但不等：`#b5bd68` 在 xterm-256 里最近的是 **i143 = (175,175,95)**。
- ref 依据：`theme/theme.ts:529` 的 `colorMode = getCapabilities().trueColor ? "truecolor" : "256color"`，
  `:206-226` 在 256 色模式下把 `#rrggbb` 换成 `[38;5;Nm`；量化公式在 `:125-199`
  （`CUBE_VALUES`/`GRAY_VALUES`/加权距离/`spread<10` 才考虑灰阶）。
  能力判定在 `tui/src/terminal-image.ts:73-74`（`COLORTERM=truecolor|24bit`）。
- pi-rs 根因：`tools/ext-host/shim/theme-colors.mjs` 的色表**硬编码真彩**，
  `runner.mjs` 的 `makeThemeStub().fg` 直接发真彩、没有降级路径 ⇒ 扩展面板颜色不随能力降级。
- 修法：宿主在 ctx 里带 `trueColor`（来自 `pi_tui::style::truecolor_supported()`，与 ref 同源判定），
  runner 在 `fg()` 里按需把 `38;2;r;g;b` 量化成 `38;5;N`（照抄 ref 的 `rgbTo256`，
  自创近似会算成 i247）。
- 改后：`i143` 与 TS **完全一致**；golden `T109-todo-alldone` 入库（`same`）。

## T2 补（真机复测"没啥变化"后的二轮修复）
- 用户反馈：装了 `acfaf6f` 后 Windows 135 列下仍溢出。`md-diff` 在 120–140 列都是 0，
  说明**问题不在 markdown 断点**，而在「渲染出的行宽超过列数」。
- 新用例 `crates/pi-tui/tests/t_width_cap.rs`：对一条与真机同构的合成长句
  （`【】（）；、%/` 数字 英文 粗体 行内代码 + `1. ` 前缀）断言
  `render_all()` 的**每一行** `visible_width <= cols`（80/90/100/110/120/125/130/133/135/140）。
  **改前 FAILED**：`cols=80 第 3 行宽 82 超出`。
- 根因：`wrap_markdown_line` 是对**整行**（已含 `" 1. "` 前缀）折行、再给续行**插入**前缀 ⇒
  `续行前缀 + 正文` 超出行宽。ref 的顺序相反：先按 `itemWidth` 折正文，再给**每一段**拼前缀
  （`markdown.ts:758-790`：`firstPrefix` / `continuationPrefix` / `itemWidth`）。
- 修法（照 ref 顺序重写 `wrap_markdown_line`）：
  列表行拆成 `first_prefix`（原样）+ `body`，`itemWidth = (cols-2) - marker`，
  正文按 `itemWidth` 折行后，首行拼 `first_prefix`、续行拼等宽空格。
  非列表行保持 `cols-2+lead`。
- 结果：`t_width_cap` 通过；`md-diff --ignore-blank` 在 **78/79/80/81/82/90/100/110/120/130/135/140/160**
  全部宽度下 `list-after-para` 与 `list-hang` 均 `different=0`（此前 80/81 等奇偶宽度会差 1 格）。
- golden 全 same：T89e/f/g/h、T106、T76、T05、T01。
