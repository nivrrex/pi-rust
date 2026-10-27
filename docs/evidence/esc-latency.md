# Esc 中止延迟

## 测量
`tools/difftest/pty_esc_latency_check.py`（每样本新起进程；中止文案按状态区分：
回合流式中止为 `Operation aborted`，工具执行中被中止为 `This operation was aborted`）。

## 结果（N=8，毫秒，p50/p99）
| 状态 | 本实现 | 对照实现 |
|---|---|---|
| 流式中止 | 27.0 / 27.0 | 33.8 / 62.3 |
| 工具执行中止 | 27.2 / 27.4 | 57.7 / 59.0 |

## 修复
工具进程终止流程原为"TERM → 无条件等待 500ms → KILL"，导致工具态中止恒为 ~550ms；
改为 TERM 后等待子进程退出、超时才 KILL（退出即返回）。
