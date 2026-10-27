# 回归结果（P8e-2 收尾，数字自述未核实）

## 文本级严格逐帧（`scripts/regress-tui.sh`）
| 轮次 | 结果 |
|---|---|
| glibc ×3（`--runs 3 --jobs 5`） | **same=189 / different=0 / nogolden=39 / runfail=0** |
| musl 静态 ×1 | **same=63 / different=0 / nogolden=13 / runfail=0** |

`nogolden` 是尚未录基线的场景（`T72` 为 print 模式不适用 TUI golden 等），不是失败。

## pty 探针（netns 内）
- `pty_retry_network_check.py`（连接不可达 / DNS 失败）：**all_ok**
- `pty_esc_latency_check.py`（流式 / 工具执行，N=4）：**all_ok**
- `pty_escape_abort_check.py`（4 配置 × N=4）：**all_ok**
- `ls_tool_check.py`（print 逐字节 + pty 调用头）：通过

## 样式级（`compare-style --style-only`，整段流口径）
| 场景 | 差异格 | 备注 |
|---|---|---|
| T01 | **0（same）** | 本批达标 |
| T06 | 23 | 版本行 dim 与帧集合差异 |
| T05 | 331 | 约 320 格为流末态不同 |
| T77 | 226 | 边框色 i242 未定位（3 次假设否证，deferred） |
| T28 / T35 | 28 / 26 | 帧数不一致（TS 6 帧、我们 5 帧） |

样式级"逐格 same"在含瞬时状态的场景里受**帧集合不一致**限制，需按内容对齐帧配对才能继续压缩；
这是工具能力限制，已记入 `LIMITATIONS.md`。
