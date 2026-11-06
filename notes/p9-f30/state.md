
## F30-S0 追加：R3 遗留取证（只取证，不改码）
预测：T45-tree-checkout 的首帧差异随**场景所用 AGENTS.md 的字节数**变化（当前工作树 AGENTS.md 与录制期不同，导致 `[Context]`/帮助区行数变化，整体位移一行）。
推翻条件：把 3500B 提取件替换进 tmp 场景副本后重跑 T45，仍缺 ` !! to run bash (no context)` 一行（即差异与 AGENTS.md 字节数无关）。
L3 基线（R3）：dark 82 same / 2 different / 20 nogolden；light 23 same / 0 different / 1 nogolden。不符即报差异。
- (b) 连跑取证（未跑满）：T45 8/10 全 different（linesG=25 / linesR=24，缺 ` !! to run bash (no context)`）；T10 7/10 全 different（pctG=1.2 / pctR=1.1）；T52 4/10 全 different（linesG=24 / linesR=23）；三轮均 0 same、0 抖动。
- (c) TS 实跑：DIFTEST_IMPL=ts 时 helperExit=3、rawBytes=1093（未到 waitScreen 目标屏），三方对照不可用；golden vs TS / golden vs pi-rs / TS vs pi-rs 三者皆 different。
- (d) run-tui 源路径：`scenarios-tui/T45-tree-checkout.json` 的 `files.AGENTS.md` = 24 字节（write 于 run-tui.mjs:75-79，落 `<tmp>/work`）；`globalFiles.CLAUDE.md` = 21 字节且 run-tui 忽略该键（run-tui.mjs:112-115 注释）；`agentFiles` 为空。把 3500B 提取件替换进 `tmp/T45-big.json` 重跑：仍 different、形状不变（25 vs 24）⇒ **预测被推翻**。
- (e) `git worktree` 检出 5cfec2e 跑 privacy-scan：**全部干净**；`git log -S"/root/Desktop/"` 只命中 `bf4c427`（恢复提交）⇒ 该字符串是恢复快照带进来的，5cfec2e 的同名文件里 0 处。未补 allowlist、未改扫描器。
- (f) 会话 cwd = /root/Desktop/pi-rust；第 131 行 read 的 path = /root/Desktop/pi-rust/AGENTS.md ⇒ 3500B 版属于 pi-rust 项目本身，全文存 notes-rebuilt/_other/AGENTS.1003-from-read.md。
