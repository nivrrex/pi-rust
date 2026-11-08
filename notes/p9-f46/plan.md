# F46 清单

起点 HEAD `eb4bb8a5`（F45 收口）。真实数据：`/root/Desktop/tty/win-real/`（只读）。

## 撤回项（真机证据否定 F45 的三条判断）
- **F45-T1「段落接列表无缺陷」撤回**：真实会话文本（`/tmp/real-md.md`，取自 win-real 的
  assistant 消息，只读、不入库）在 width 110 下 `md-diff` 报 `r20` 差异 ——
  TS 紧接列表无空行、pi-rs 多一行。最小复现（合成）：
  `普通段（三份）：\n- 甲项一` ⇒ TS 1 行、pi-rs 2 行。
- **F45-T3「会话缺字段/旧进程」撤回**：真实会话**有** `model_change{provider:"mw-auto",
  modelId:"auto"}`，settings 也是 `mw-auto/auto`；pi-rs 本地 `-c` 也能显示 `(mw-auto) auto`。
  真机 `no-model` 的真因见 U2。
- **F45-T4「TS 截图非 Windows」撤回**：真实 settings 是 `theme: light`，而 shim 色表
  **只由 dark.json 生成** ⇒ light 主题下扩展颜色全错。真因见 U3。

## U2 `-c` 的 no-model / 0.0%（已修，根因实证）
- 真机底栏是**扩展页脚**（`pi-ai-summary/src/footer-cny.ts`）：
  `:342` `const modelName = model?.id || "no-model"`、
  `:274` `contextWindow = (ctx.model as any)?.contextWindow ?? 0`。
- pi-rs：`set_ctx_model_object` **只在「新建会话」分支**调用（`main.rs:1549`），
  `-c`/`--session` 走 `main.rs:1476-1487` 的恢复分支 —— 那里**从未**写 `ctx.model`
  ⇒ 扩展读到的 `ctx.model` 是 null ⇒ 页脚 `no-model`、`contextWindow=0` ⇒ `0.0%`。
  原生页脚走 `state.model`（F45 已修），所以真机上**只有扩展页脚**表现为 no-model。
- 修法：恢复分支补 `set_ctx_model_object(...)`（用会话里恢复出的 provider/model）。
- 用例 `crates/pi-cli/tests/p9_f46_resume_ctx_model.rs`：**改前 FAILED**
  （`恢复会话时扩展的 ctx.model 为空`），改后 ok。
- 另：T108 场景 `no-model`/`(mw-auto)` 的差异同源于此。

## U3 light 主题下 Todo 颜色（已修，根因实证）
- 真实 `settings.json` 的 `theme` 是 **light**；ref `theme/theme.ts:529` 按主题名取主题对象。
- pi-rs：`tools/ext-host/shim/theme-colors.mjs` 的注释写明「由 ref `theme/dark.json` 生成」，
  且 `runner.mjs` 的 `makeThemeStub().fg` 只用这份表 ⇒ light 主题下扩展拿到**深色**调色板
  （`success` = `#b5bd68`），而 light 的 `success` 是 `#588458`。
- 修法：
  - 新增生成器 `tools/ext-host/gen-theme-colors.mjs`（从 `theme/{dark,light}.json` 生成
    `FG_COLORS/_LIGHT`、`BG_COLORS/_LIGHT` 与 `fgColorsFor(themeName)`），重生成产物（56 键不变）。
  - 宿主在 ctx 里带 `themeName`（`ext_tools::update_ctx_theme`，在**建宿主前**按 settings 写入）。
  - `makeThemeStub` 按 `hostCtx.themeName` 选表。
- 用例 `crates/pi-cli/tests/p9_f46_light_todo_color.rs`：**改前 FAILED**（仍在用 `38;2;181;189;104`），改后 ok。
- golden：新增 `T112-todo-light`（TS 录制，light 下 `✓ 全部完成` 为 256 色 i65）；`T109-todo-alldone`
  （dark）无回归。

## U1 段落接列表的空行（**未修，已撤回实现**）
- 最小复现已拿到（见上）。ref `markdown.ts:487-497`：
  `case "paragraph"` 末尾 `if (nextTokenType && nextTokenType !== "list" && nextTokenType !== "space") lines.push("")`。
- **pulldown 不提供 `nextTokenType`**：实测 `段\n- 甲` 与 `段\n\n- 甲` 的**事件序列完全相同**
  （都是 `Start(Paragraph)/End(Paragraph)/Start(List)/Start(Item)`），
  「源码里有没有空行」不在事件流里。
- 试过三种补法均失败并已全部 `git checkout` 撤回：
  ①无条件不补 ⇒ T76（代码块后接列表**要**空行）、T106（段落+空行+列表**要**空行）回归；
  ②按"上一块类型"标记 ⇒ 同上，且 T89f/T89h 回归；
  ③列表首行行内哨兵 ⇒ 让 pulldown 把该层判为 loose（项间多空行），T05/T113 回归；
     改行尾哨兵 ⇒ 标记泄漏进正文，全部 markdown golden 变成 different。
- 结论：要正确实现必须**自建块级扫描**（按源码行判断"段落块结束 + 下一非空行是列表首行"），
  工作量与风险都超出单批；`notes/questions.md` 已登记，建议单列批次。
