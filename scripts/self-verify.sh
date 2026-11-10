#!/bin/sh
# F53/T6：一条命令跑"真实配置自动化验收"。
#
#   1. 生成脱敏真实配置副本（`tmp/realcfg`）
#   2. T4：TS × pi-rs 逐步屏幕对拍（real-parity 全矩阵）
#   3. T5：每步不变量断言（宽度 / 控制字符 / 页脚分支与 thinking / 内容保全）
#   4. 收尾删除 `tmp/realcfg`（真实配置副本不留在仓库里）
#
# 退出码 0 = 全部通过；非 0 = 有格子不同（明细在 `tmp/real-parity/summary.txt`）。
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO"

"$REPO/tools/difftest/mk-realcfg.sh" >/dev/null
status=0
"$REPO/tools/difftest/real-parity.sh" "$@" || status=$?
summary="$REPO/tmp/real-parity/summary.txt"
if [ -f "$summary" ]; then
  printf '=== self-verify 摘要 ===\n'
  grep -E '^(===|step-|compare-steps|assert-steps|real-parity)' "$summary" | tail -40
fi
rm -rf "$REPO/tmp/realcfg"
exit "$status"
