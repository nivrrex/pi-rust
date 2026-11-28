# 会话级重试

## 判据
错误文案匹配（先排除订阅/额度上限与证书/TLS 类），命中瞬时错误才重试：
`timeout`、`connection refused`、`connection reset`、`broken pipe`、
`failed to lookup address`、`dns error`、`network is unreachable`、`5xx`、`429`、
`stream ended without …` 等。

## 退避与显示
- 指数退避 2s / 4s / 8s，最多 3 次。
- 状态行 `Retrying (n/3) in Xs... (escape to cancel)`，倒计时逐秒递减、不出现 `in 0s`。
- 中间轮次的失败**不再单独宣告**（避免每次重试多一个错误块）；
  最终放弃时先移除状态行，再打 `Retry failed after N attempts: <err>`。
- 期间按 Esc 可取消。

## 复现
- `tools/difftest/pty_retry_network_check.py`：netns 内分别指向不可路由地址与不存在的域名，
  断言出现重试、约 14s 后失败（2+4+8）。
- `tools/difftest/pty_retry_check3.py`：cancel / success / exhausted 三条路径。
