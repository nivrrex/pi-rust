# 回归结果（P8e-2，数字自述未核实）

## 文本级严格逐帧（`scripts/regress-tui.sh --runs 3 --jobs 5`）
- **same=183 / different=6 / nogolden=39 / runfail=0**（74 个有 golden 的场景 ×3）。
- 6 个 different 全部来自两个场景：`T74-retry-success` 与 `T75-bash-60`（各 3 次），
  两者都是"整屏比 TS 少/多 1 行"的同类差异（内容逐行一致、只是起点差一行），已登记
  `notes/limits.md`；其余 68 个场景 ×3 全部 same。
- `nogolden` 是尚未录基线的场景（`T72` 为 print 模式不适用 TUI golden，其余见 B 项）。

## 样式级（`tools/difftest/compare-style.mjs --style-only`，最终代码）
| 场景 | 差异格 | 背景 | 前景 | 粗体 | 暗淡 |
|---|---|---|---|---|---|
| T01 | 29 | 0 | 6 | 0 | 23 |
| T28-thinking-selector | 3 | 0 | 0 | 2 | 3 |
| T35-model-thinking | 3 | 0 | 0 | 0 | 3 |
| T77-thinking-levels | 226 | 0 | 223 | 0 | 3 |
| T06 | 332 | 0 | 306 | 104 | 26 |
| T05 | 433 | 0 | 433 | 2 | 23 |

背景色已全部对齐；剩余差异集中在"同文本不同身份"的行（如都是 `────` 的边框行）与
部分组件的着色键（详见 `LIMITATIONS.md`）。
