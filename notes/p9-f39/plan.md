# F39 清单

起点 HEAD `9f6c3ec3`。

- [x] M0 补报 F38（**F38 未做，本批补做**）：
      - K3b 真实 pi-handoff-gate（脱敏副本）：工具**已执行**；②Todo 部件原先完全缺失，
        根因是 runner 的 `ctx.ui` **没有 `theme`**（ref `types.ts:276-282`），扩展 `ctx.ui.theme.fg(…)`
        抛错后被自身 try/catch 吞掉 ⇒ 修后 ` Todo ▱▱▱▱ 0/3` 与 TS 一致（`5296f28`）。
        ①工具行仍是内置渲染（` todo set` + 展开 JSON），差异见下。
      - K4 真实 pi-ai-summary：工具行/thinking 行是**原型补丁**，跨进程架构下不生效（见下节），
        需单列批次；底栏 `↯` 未出现，待查 `installCnyFooter` 早退条件。
- [x] M1 启动屏列表着色/折行：
      - 合成夹具 `T98-startup-lists-wrap`（80×40，12 skill + 12 扩展，均折 4 行）与
        `T99-startup-lists-wrap-120`（120×40，折 2 行）。
      - 根因三条：①`[Skills]`/`[Extensions]` 列表行原先**整行样式**（行级样式跳过前导空格），
        且非末行**不补齐到整宽** ⇒ 补位空格无色（TS 中间行 `0..cols` 全 dim）；②`Press ctrl+o …`
        与 onboarding 行同样缺补位；③键位提示续行（`more`）应是 muted(i244) 而非无色。
      - 修法：新增 `push_dim_wrapped`（非末行补齐整宽 + span 从第 1 列起）、
        `push_compact_list` 返回行区间并改用 span（覆盖前导空格）、
        `header_spans` 增加 `in_keyhint_section` 处理续行、`pi v…` 行加 `dim`。
      - 结果：T98 逐格差异 **11 → 1 行**（剩余 1 行为夹具路径不同，非样式差异）；
        T99 **6 → 2 行**（1 行同路径差异，1 行 `pi v…` 行的 dim 属性）。
      - 既有 golden 无回归：T01/T05/T90/T100/T91/T92/T89f 全 same。
- [x] M2 退出后首个按键字面回显 `^[[A`（时间线取证 + 收尾窗口修复）
      - 夹具 `tools/difftest/exit-timeline.py`：同一 pty 上先跑被测程序，返回时刻为 t=0，
        记录最后写 tty 时刻、`/proc/*/cmdline` 里的 `runner.mjs` 存活（每 25 ms）、
        termios 快照（t=0 以及 shell 设 raw 后 0–500 ms 每 10 ms）、以及"不改 termios 时写入 `ESC[A`"
        是否被字面回显。
      - 时间线（abbrev）：
        | 指标 | TS | pi-rs 改前 | pi-rs 改后 |
        | --- | --- | --- | --- |
        | 最后一次写 tty → 进程返回 | 40.2 ms | **161.1 ms** | **60.3 ms** |
        | 退出时 termios | cooked（echo/icanon/isig/iexten 全开） | 同 | 同 |
        | `runner.mjs` 在 500 ms 内存活 | 无 | 无 | 无 |
        | 不改 termios 时写 `ESC[A` | `^[[A`（字面回显） | `^[[A` | `^[[A` |
        | shell 设 raw 后 500 ms 内被改回 | 否 | 否 | 否 |
      - 结论：**候选②成立**——`restore()` 的 drain idle 窗口是 **150 ms**（F3 时代的兜底值），
        是 ref `terminal.ts:384` `drainInput(maxMs=1000, idleMs=50)` 的 3 倍；这段时间 tty **仍是 raw**，
        用户退出后立刻按的键会在队列里等，返回后被 shell 读走/回显。宿主晚退出（候选①）不存在
        （两侧都没有 `runner.mjs` 残留）；应答残留（候选③）两侧一致且为零。
      - 修法：idle 150 → 50 ms（`crates/pi-cli/src/tui.rs:121`）。
      - 新测 `crates/pi-cli/tests/p9_f39_exit_timeline.rs`：改前 FAILED（gap 160.7 ms），改后连跑 3 次通过；
        另断言"退出后 500 ms 内无 `runner.mjs` 存活""shell 设 raw 后 termios 不被改回"。
      - 既有 golden 无回归：T01/T05/T90/T89i/T100 全 same。
- [x] M3 收口（不做发布包）
      - L2 全量：workspace **1082 passed / 0 failed**；clippy 0；`lint-banned` OK；
        precommit **11/11**；regress **dark 100/0/22**、**light 23/0/1**；
        privacy-scan 干净（`tmp/realcfg` 已删）；readonly 一致；check-orphans 0；
        `git status` 仅 `?? AGENTS.md`。
      - 连带修正：`p9_f3_kitty_release.rs` 的两个用例原先钉 60/120 ms —— 那是 F3 时代
        idle 放宽到 150 ms 的产物。按 ref 收回 50 ms 后改为 40/45 ms（窗口内仍被吃掉，
        窗口外泄漏是 **ref 的既有行为**，不是本实现引入）。

## M0 补报 F38（K3b 真实 pi-handoff-gate）——**F38 未做，本批补做**
- 夹具：`scenarios-tui/T101-real-hg-todo.json`，扩展内容为 `~/.pi/agent/extensions/{pi-handoff-gate,pi-ai-summary}`
  的**脱敏副本**（models.json 的 13 处 baseUrl→`127.0.0.1:1`、apiKey→DUMMY，收口前删除 `/tmp/realcfg`）；
  模型响应 `fixtures/sse/openai/tool-todo.sse`（`todo` 工具，`action=set` 三项）。
- 差异表（100×34，dark）：
  | 项 | TS | pi-rs |
  | --- | --- | --- |
  | 工具调用行 | `✓ todo {"action":"set","items":[…] } · 0.0s` | ` todo set` + 展开的 JSON 参数 + 结果三行 `[~] #1 …` |
  | Todo 部件 | ` Todo ▱▱▱▱ 0/3` / ` ▶ #1 甲任务` / ` · #2 …`（belowEditor，4 行） | **无**（`setWidget` 未生效） |
  | 启动屏 `[Extensions]` | `  pi-ai-summary, pi-handoff-gate` | 同 |
  | 底栏 | `↑50 ↓14 0.0%/128k (auto) ↯ 2000.0/s …` | `↑30 ↓10 0.0%/128k (auto) …`（工具输出未折叠 ⇒ 计数不同源） |
- 结论：工具**已执行**（结果文本与原版 `renderTodos` 一致），差异在**渲染层**：①`renderCall`/`renderResult`
  未生效（TS 用 `✓ name {compact args} · 0.0s`，我们的扩展工具走了内置展开渲染）；
  ②工具执行期调用的 `ctx.ui.setWidget` 未到达 TUI（`session_start` 期调用是生效的，见 T96）。
  后者疑与"`ext.tool.invoke` 等待应答期间 runner 发出的 `host.ui.setWidget` 请求被丢"有关，
  待下一步用 `PI_EXT_DEBUG_WIRE` 取证。

### M0/K4 真实 pi-ai-summary 对拍
- 同一夹具（T101，真实两扩展副本 + `todo` 工具调用）下再比："工具行 / thinking 行"。
- | 项 | TS | pi-rs |
  | --- | --- | --- |
  | 工具行（折叠） | `✓ todo {"action":"set","items":[…] } · 0.0s` | ` todo set` + 展开 JSON + 结果三行 |
  | thinking 行 | 未触发（夹具无 reasoning 增量） | 同 |
  | 底栏 `↯ 2000.0/s` | 有 | **无** |
- **根因（结构差异，不是遗漏字段）**：`pi-ai-summary` 的工具行与 thinking 行是**原型补丁**
  （`src/adapters/tool-render.ts` 的 `ToolExecutionComponent.prototype.render`、
  `thinking-label.ts` 的 `AssistantMessageComponent.prototype`）。ref 里扩展与 TUI 同进程 ⇒ 补丁直接改渲染；
  我们把扩展放在独立 node 进程、TUI 在 Rust ⇒ **补丁对渲染零影响**（`shim/pi-coding-agent.mjs:66-77`
  只给了空壳类）。
- 要真正对齐，需要"宿主把工具行渲染委托给 runner"（类似 F38/K4 的 `ext.render.*`，但对象是
  **原型补丁后的 ToolExecutionComponent**）：runner 在 `ext.render.toolrow` 里用被补丁的类
  构造实例并 `render(width)`，宿主按 `(tool, args 摘要, 结果摘要, expanded, width, theme)` 缓存。
  工作量估 200–300 行，跨 runner/Rust/tui 三处，**建议单列批次**。
- 底栏 `↯`：`footer-cny.ts` 走公开 API `ctx.ui.setFooter`，本应生效（F38/K4 已支持按需重渲染），
  但实测未出现 —— 需查 `installCnyFooter` 的早退条件（此前 F36 也登记过同一疑点）。
