
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
- (c) 三方逐行：T45 → golden == TS live（25 行逐行一致），pi-rs 24 行、第一处不同行号 **0**（缺顶部 ` !! to run bash (no context)`，实为 pi-rs 少一个空行导致整屏上移）；T10 → golden == TS live（27 行一致），pi-rs 第一处不同行号 **25**（`1.2%` vs `1.1%`）；T52 → TS live 与 golden 也不一致（22 vs 24 行，第一处不同行号 0），pi-rs 23 行。
- (d) T10 百分比口径：ref `footer.ts:110-113`（`contextWindow/percent` 取自 `session.getContextUsage()`），`agent-session.ts:3858-3904`（`estimateProjectedContextTokens(...).tokens / contextWindow * 100`）。pi-rs 对应 `crates/pi-cli/src/tui.rs` 的 `TuiEvent::Usage` 分支（percent 由 `context_tokens`/`system+text` 估算），`crates/pi-tui/src/lib.rs:869-880` 负责格式化。
- (e) T45 文案来源：ref `interactive-mode.ts:955-990`（`expandedInstructions` 数组，`!!` 在 :978）；pi-rs `crates/pi-tui/src/lib.rs:1847-1849` 有同一行。差异不在该行本身，而在其上方少一个空行（golden 第 18 行为空、pi-rs 第 18 行即分隔线），产生空行的组件行号**未定位**。
