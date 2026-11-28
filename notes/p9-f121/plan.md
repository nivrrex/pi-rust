# F99–F120 批次收口（20 个提交，HEAD `bcaf6da`）

## L3 结果（全部实跑）
- `scripts/precommit.sh`：**18 项全部通过**（含本轮新增的 P7-6 判定自检）。
- `cargo test --workspace`：**289 组 ok / 0 失败**。
- `scripts/regress-tui.sh`：golden 覆盖 **125 个场景，125 same / 0 diff / 0 缺失**。
  **light 维度 27/27 same、truecolor 7/7 same**（F122 补齐）。此前 truecolor
  的 7 个 golden **没有任何驱动脚本**，只能手工 `run-tui.mjs` + `COLORTERM=truecolor`
  逐个跑，长期没被自动回归覆盖；现 `THEME=truecolor` 可用，且只跑有 golden 的
  那 7 个（不再为 141 个无 golden 的场景白跑）。
- `tools/difftest/real-parity-linux.sh`：**8 格 different，全部同源**——
  8 个格 × 8 处 `defaultProvider/defaultModel … is not in the model registry` 警告，
  即 F63/D 那条"用户要求的偏离"在真实错配配置下的必然代价。无新增差异。
- `scripts/check-no-stray.sh` / `check-orphans.sh`：无残留进程。

## 本批做了什么（按主题）
1. **主题/终端**：`theme` 的 auto 形式此前被 `BUILTIN.contains` 过滤掉、等于没生效（F89）；
   OSC 11 查询**有解析器没有发送方**（F100）、应答用于选主题（F101/F102）；
   `theme.bg` 的背景色表此前是"前景色加 48 前缀"自造的，ref 只有 7 个 `ThemeBg` 键（F107）。
2. **扩展 UI 组件**：`SelectList` 完全忽略传入的 5 个样式函数（F108）；
   `Text` 三处实现差异（F109）；`renderResult`/Ctrl+O 其实是好的，
   只是 LIMITATIONS 记录过时（F110）。
3. **续跑**：`agent_before_settle` 的 `should_continue` 真正实现（F106），
   此前被三次失败卡住；这次用"前置声明绑定"绕开了结构搬移。
4. **诊断呈现**：扩展加载失败（F115）与 6 处运行期告警（F116）此前直接 `eprintln!`，
   TUI 下会把首屏顶掉。
5. **检查器自身**：`check-no-stray.sh` 把 pty-helper 残留判成"人工会话"放过（F118），
   补了判定自检（F119）并接进 precommit（F120），同时去掉标题里写死的项数。
6. **验证基建**：会话双向互通固化成 `scripts/session-interop-check.sh` 并借此修了
   `session-dump` 漏掉 system 消息（F103）；`after_provider_response` 的手工探针
   固化成用例（F114）。

## 记录纠错
- LIMITATIONS 里 real-parity 由"4 格"更正为"8 格同源"（F99）。
- `compare-style` 的 124 个"样式差异"查明是**退出尾部字节造成的假阳性**（F99 记录）。
- `renderResult` 未接进工具块、Ctrl+O 对拍未通过——两条均已过时（F110）。
- `after_provider_response` 的待验证项用真实 HTTP 流关闭（F111）。

## 未解决 / 未验证（如实保留）
- **Case 2 溢出窗口无法构造**：`overflow_outcome` 需要 `usage_matches_projection`，
  mock 写死假 usage 通不过 ⇒ F112 的修复只有单元测试作证（F113 记录）。
- **F116 那 6 处运行期告警无端到端用例**：触发条件（宿主写会话失败/路径 10s 未就绪）
  构造不出；只验证了"正常回合 stderr 为 0 行"这个否定面（F117）。
- `Box`/`Markdown`/`Table` 仍不支持（已装扩展中无使用）。
