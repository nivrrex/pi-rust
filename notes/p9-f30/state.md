
## F30-S0 追加：R3 遗留取证（只取证，不改码）
预测：T45-tree-checkout 的首帧差异随**场景所用 AGENTS.md 的字节数**变化（当前工作树 AGENTS.md 与录制期不同，导致 `[Context]`/帮助区行数变化，整体位移一行）。
推翻条件：把 3500B 提取件替换进 tmp 场景副本后重跑 T45，仍缺 ` !! to run bash (no context)` 一行（即差异与 AGENTS.md 字节数无关）。
L3 基线（R3）：dark 82 same / 2 different / 20 nogolden；light 23 same / 0 different / 1 nogolden。不符即报差异。
- (b) 连跑取证（未跑满）：T45 8/10 全 different（linesG=25 / linesR=24，缺 ` !! to run bash (no context)`）；T10 7/10 全 different（pctG=1.2 / pctR=1.1）；T52 4/10 全 different（linesG=24 / linesR=23）；三轮均 0 same、0 抖动。
- (c) TS 实跑：DIFTEST_IMPL=ts 时 helperExit=3、rawBytes=1093（未到 waitScreen 目标屏），三方对照不可用；golden vs TS / golden vs pi-rs / TS vs pi-rs 三者皆 different。
- (d) run-tui 源路径：`scenarios-tui/T45-tree-checkout.json` 的 `files.AGENTS.md` = 24 字节（write 于 run-tui.mjs:75-79，落 `<tmp>/work`）；`globalFiles.CLAUDE.md` = 21 字节且 run-tui 忽略该键（run-tui.mjs:112-115 注释）；`agentFiles` 为空。把 3500B 提取件替换进 `tmp/T45-big.json` 重跑：仍 different、形状不变（25 vs 24）⇒ **预测被推翻**。
- (e) `git worktree` 检出 5cfec2e 跑 privacy-scan：**全部干净**；`git log -S"/root/Desktop/"` 只命中 `bf4c427`（恢复提交）⇒ 该字符串是恢复快照带进来的，5cfec2e 的同名文件里 0 处。未补 allowlist、未改扫描器。
- (f) 会话 cwd = /root/Desktop/pi-rust；第 131 行 read 的 path = /root/Desktop/pi-rust/AGENTS.md ⇒ 3500B 版属于 pi-rust 项目本身，全文存 notes-rebuilt/_other/AGENTS.1003-from-read.md。

### F30-S0b（TS 实跑与逐行 dump）
预测：TS 实跑（DIFTEST_IMPL=ts、TS_PI_ENTRY=/root/Desktop/pi-ts-0.87.1/dist/bundle/cli.js）下 T45 首帧应含 ` !! to run bash (no context)`，即 golden 与 TS 一致、差异只出在 pi-rs。
推翻条件：TS 实跑同样缺该行 ⇒ golden 与当前 TS 环境不一致，不能归因 pi-rs。
- (a) 连跑终态：T45 **10/10 different**（linesG=25/linesR=24）；T10 **10/10 different**（pctG=1.2/pctR=1.1，两处长度均 27）；T52 **10/10 different**（linesG=24/linesR=23）；三轮 0 same、参数零抖动 ⇒ 确定性差异。
- (b) TS 失败根因（已证实）：`TS_PI_ENTRY=/root/Desktop/pi-ts-0.87.1/...` 时 node 报 `ERR_MODULE_NOT_FOUND: Cannot find package '@earendil-works/chord'`（该目录 node_modules 为空）。`run-tui.mjs:205-216`：`DIFTEST_IMPL=ts` 时 `defaultCmd = node $TS_PI_ENTRY`；`TUI_CMD` 若设置会**整体覆盖**该命令（我此前的 TUI_CMD=node 导致跑成 REPL）。`pty-helper.py:202-216`：`helperExit=3` = `waitStrip` 超时。改用 pnpm 包路径作为 TS_PI_ENTRY 后 helperExit=0。
- (c) 三方逐行（T52 为完整跑完后的终值，此前 22 行是运行中途读取）：T45 → golden == TS live（25/25 逐行一致），pi-rs 24 行，第一处不同行号 **0**（顶部缺 ` !! to run bash (no context)`，成因是 pi-rs 少一个空行导致整屏上移）；T10 → golden == TS live（27/27），pi-rs 27 行，第一处不同行号 **25**（footer `1.2%` vs `1.1%`）；T52 → golden == TS live（24/24），pi-rs 23 行，第一处不同行号 **0**。三处均为 pi-rs 侧差异。
- (d) T10 百分比口径：ref `footer.ts:110-113`（`contextWindow/percent` 取自 `session.getContextUsage()`），`agent-session.ts:3858-3904`（`estimateProjectedContextTokens(...).tokens / contextWindow * 100`）。pi-rs 对应 `crates/pi-cli/src/tui.rs` 的 `TuiEvent::Usage` 分支（percent 由 `context_tokens`/`system+text` 估算），`crates/pi-tui/src/lib.rs:869-880` 负责格式化。
- (e) T45 文案来源：ref `interactive-mode.ts:955-990`（`expandedInstructions` 数组，`!!` 在 :978）；pi-rs `crates/pi-tui/src/lib.rs:1847-1849` 有同一行。差异不在该行本身，而在其上方少一个空行（golden 第 18 行为空、pi-rs 第 18 行即分隔线），产生空行的组件行号**未定位**。

### S0b 续：50 行视口下的全屏对照
- 用法：把 T45 场景复制成 `tmp/T45-r50.json`（仅 `tui.rows=50`，仓库文件不动），TS（pnpm 包）与 pi-rs 各跑一次。
- 结果：TS 42 行 / pi-rs 42 行，**唯一差异在第 25 行**：TS `  <TMP>/work/AGENTS.md`（绝对路径），pi-rs `  AGENTS.md`（相对路径）。其余 41 行逐行一致。
- 30 行视口下 pi-rs 少 1 行（25 vs 24）与该差异的因果关系**未证实**：两者在同一 42 行内容下仅 1 行不同，行数相同。
- ref 依据：`formatContextPath`（`interactive-mode.ts:1342-1350`）——路径在 cwd 内返回相对路径，否则 `formatDisplayPath`（home→`~`，其余绝对）；`getCwdRelativePath`（`utils/paths.ts:108-117`）做"是否在 cwd 内"判定。TS 实跑输出了绝对路径 ⇒ 在其环境里该判定返回 undefined（具体原因未证实）。

### S0b 续 2：视口高度实验（rows=31）
- TS 26 行 / pi-rs 25 行；两者**底部三行完全相同**（`<TMP>/work`、footer、空），差异只在**顶部起点**：TS 首行 ` ! to run bash`，pi-rs 首行 ` !! to run bash (no context)`。
- 结合 rows=50 的结论（内容 42 行两边等长，仅第 25 行 `[Context]` 路径写法不同）⇒ **pi-rs 并没有少渲染内容行**，而是在 30/31 行视口下**可见起点比 TS 晚一行**（滚动/裁剪锚点不同）。
- 状态：「内容未缺失」已证实；「滚动锚点为何差一行」未证实（未定位到具体渲染代码）。

### S0b 续 3：31 行整帧（未裁剪）对照
- 两边整帧都是 31 行，**从「Pi can explain…」往下逐行相同（含底部 9 行：Tool output / 空 / 分隔线 / 空 / 分隔线 / cwd / footer / resume 提示 / 空）**。
- 唯一差别是**滚动起点**：TS 顶行 ` ! to run bash`，pi-rs 顶行 ` !! to run bash (no context)`，此后整体差一行；第 29 行 resume 提示内的临时目录名不同（各自 mkdtemp，属正常）。
- 与 rows=50 的结果（两边内容 42 行、仅 `[Context]` 路径一行不同）合并看：内容总行数在无滚动时相等，滚动时可见起点差一行 ⇒ 属**滚动/锚点**差异，不是内容缺失。
- 未证实：具体是哪一处渲染逻辑（`crates/pi-tui/src/lib.rs` 的整屏组装 vs 视口高度预算）导致起点差一行；未定位到行号，故不改码。

### S0b 续 4：渲染入口与预算公式排查
- 实测帧结构（rows=31，未裁剪）：TS 与 pi-rs 都是 **23 行会话区 + 8 行 chrome**（chrome = 空 / 上下边框 / 空 / cwd / footer / resume 提示 / 空），仅起点错一行。
- `crates/pi-tui/src/lib.rs:2540-2555` 的 `render()` 里有预算公式 `capacity = rows - (chrome_lines + 4)`（rows=31 ⇒ 22），与实际 23 行会话区**不符**——但该函数**不是** TUI 实际使用的入口。
- 实际入口：`crates/pi-cli/src/tui.rs:4143` 调用 `pi_tui::render_all_marker_cached_styled(state, cols, rows, cache)`。裁剪/滚动发生在这条路径上，具体行号**未定位**。
- 因此"滚动起点差一行"仍未归因；已排除的假设：内容缺失（rows=50 等长）、布局预算公式（live 不走 `render()`）。
