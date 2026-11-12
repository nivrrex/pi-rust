#!/bin/bash
# F56/P：禁止脚本里出现"按名字/端口/模式批量杀进程"的写法。
#
# 背景：F55 期间我用 PID 清理测试进程时误杀了用户桌面进程（openbox）。
# 两类写法都被禁止：
#   1. `pkill` / `killall`（**天然按名字匹配**，最危险）；
#   2. `kill $(pgrep …)` / `kill -9 $(pgrep …)`（把按名匹配的结果直接喂给 kill）。
#
# 唯一允许的方式：`tools/difftest/safe-kill.sh --pidfile <自己写的 pidfile>`——
# 它校验 pid + starttime + cmdline 三元组，并按进程组发信号。
#
# 例外（白名单，按行首路径匹配）：
#   - `tools/difftest/safe-kill.sh` 自身（它就是实现）；
#   - `scripts/check-kill-safety.sh` 自身（模式字符串出现在本文件里）。
set -uo pipefail

REPO=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO" || exit 2

FAILED=0
report() {
  echo "  $1"
  FAILED=1
}

# 只看"会被执行"的脚本/命令片段，不看文档。
# 已跟踪 + **未跟踪**（新写的脚本还没 add 时也必须检查；用 `--others --exclude-standard`
# 避免把 tmp/ 下的临时文件算进来）。
files=$(
  {
    git ls-files '*.sh' '*.mjs' '*.js' '*.py' '*.bat' 2>/dev/null
    git ls-files --others --exclude-standard '*.sh' '*.mjs' '*.js' '*.py' '*.bat' 2>/dev/null
  } | grep -v '^docs/' | sort -u
)
[ -n "$files" ] || files=$(find tools scripts -type f \( -name '*.sh' -o -name '*.mjs' -o -name '*.js' -o -name '*.py' \) 2>/dev/null)

while IFS= read -r file; do
  [ -n "$file" ] || continue
  case "$file" in
    tools/difftest/safe-kill.sh|scripts/check-kill-safety.sh) continue ;;
  esac
  [ -f "$file" ] || continue
  # 去掉注释行再匹配，避免"文档里提到 pkill"被误判。
  body=$(grep -vE '^[[:space:]]*#' "$file" 2>/dev/null || true)
  if printf '%s\n' "$body" | grep -qE '(^|[^[:alnum:]_])(pkill|killall)([[:space:]]|$)'; then
    report "禁用 pkill/killall：$file"
    printf '%s\n' "$body" | grep -nE '(^|[^[:alnum:]_])(pkill|killall)([[:space:]]|$)' | head -3 | sed 's/^/      /'
  fi
  if printf '%s\n' "$body" | grep -qE 'kill[[:space:]]+(-[A-Za-z0-9]+[[:space:]]+)?\$\([[:space:]]*pgrep'; then
    report "禁用 kill \$(pgrep …)：$file"
    printf '%s\n' "$body" | grep -nE 'kill[[:space:]]+(-[A-Za-z0-9]+[[:space:]]+)?\$\([[:space:]]*pgrep' | head -3 | sed 's/^/      /'
  fi
done <<<"$files"

# 另外要求：safe-spawn/safe-kill 自身的用例必须存在且能跑（防止工具被"顺手删掉"）。
if [ ! -x tools/difftest/test/test-safe-tools.sh ]; then
  report "缺少 tools/difftest/test/test-safe-tools.sh（安全工具的行为用例）"
fi

if [ "$FAILED" -ne 0 ]; then
  echo "check-kill-safety: 失败（改用 tools/difftest/safe-kill.sh --pidfile）"
  exit 1
fi
echo "check-kill-safety: OK（无按名/按模式的批量杀进程写法）"
