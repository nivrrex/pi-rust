# TUI 对拍（文本级 + 样式级）

## 方法
1. 场景在 `scenarios-tui/*.json`：声明 mock 的 SSE 响应、按键步骤、快照点。
2. `tools/difftest/run-tui.mjs` 驱动 pty 录制两侧（`DIFTEST_IMPL=ts|pi-rs`），产出 `raw.bin`。
3. 文本级：`tools/difftest/compare-tui.mjs`（**丢弃 SGR**，只比文本与布局）。
4. 样式级：`tools/difftest/compare-style.mjs`（逐单元格 字符/前景/背景/粗体/斜体/暗淡/反显；
   `--style-only` 只比样式通道）。

## 结论
- 文本级：全部有 golden 的场景严格逐帧 same（含本批新增 `T75`/`T76`/`T77`）。
- 样式级：块级配色已对齐；**行内多色**（一行里混 `dim`/`muted`/`accent`）需要 span 级渲染，
  已实现 span 通道，剩余差异登记在案（未放宽判据）。

## 注意
- 内容溢出的场景里视口容量按 `rows - 9` 裁剪（`pi-tui` 的 `render()`）；早期按 `rows - 8`
  会整体少滚一行。
- 含固定 `delay` + 多回合的场景对机器负载敏感，回归脚本把它们放进串行列表。
