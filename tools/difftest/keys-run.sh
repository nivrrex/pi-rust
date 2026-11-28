#!/bin/sh
# F60：用 `safe-spawn.sh` + `safe-kill.sh` 在真 pty 里驱动 pi-rs 的按键序列。
#
# 存在的理由：`PI_TEST_KEYS` 驱动的"无响应"类缺陷需要**可脚本化**的前后台控制
# （能判定"进程还活着但屏幕不动"），而 `run-real.sh` 只服务于 TS/pi-rs 逐步对拍，
# 不带超时/存活判定。本脚本把 pty-helper 包在安全启动器里，超时后按进程组清理。
#
# 用法：
#   keys-run.sh --out DIR [--cols N] [--rows M] [--keys "a,enter,b"]
#               [--timeout SEC] [--no-keys-env] -- CMD [ARGS...]
#
# 产物：DIR/raw.bin、DIR/steps/step-N.txt、DIR/code.txt、DIR/helper.log、
#       DIR/liveness.txt（每 200 ms 采样一次"raw.bin 是否还在增长"）
#
# 退出码：0 = 被测进程自行退出；3 = 超时（被 safe-kill 收掉）；其它 = helper 错误。
set -eu
REPO=$(cd "$(dirname "$0")/../.." && pwd)

OUT=""
COLS=120
ROWS=40
KEYS=""
TIMEOUT=300
KEYS_ENV=1
while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --cols) COLS="$2"; shift 2 ;;
    --rows) ROWS="$2"; shift 2 ;;
    --keys) KEYS="$2"; shift 2 ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    --no-keys-env) KEYS_ENV=0; shift ;;
    --) shift; break ;;
    *) break ;;
  esac
done
[ -n "$OUT" ] || { echo "keys-run: 缺 --out" >&2; exit 2; }
[ $# -gt 0 ] || { echo "keys-run: 缺命令" >&2; exit 2; }

rm -rf "$OUT"
mkdir -p "$OUT/steps"
printf '[]\n' > "$OUT/noop.json"

PIDFILE="$OUT/pidfile"
CODE="$OUT/code.txt"
RAW="$OUT/raw.bin"

# 按键经 pty-helper 的 `--script` 之外**只**走 PI_TEST_KEYS：两侧语义一致
# （pi-rs 的钩子在首帧稳定后逐键注入，走与真实按键相同的 `bytes → TuiEvent` 路径）。
if [ "$KEYS_ENV" = "1" ]; then
  export PI_TEST_KEYS="$KEYS"
fi
export PI_TEST_DUMP="$OUT/steps"
export PI_TEST_TAIL="$ROWS"

"$REPO/tools/difftest/safe-spawn.sh" --pidfile "$PIDFILE" --out "$OUT/helper.log" -- \
  env "PI_TEST_KEYS=${PI_TEST_KEYS:-}" "PI_TEST_DUMP=$PI_TEST_DUMP" "PI_TEST_TAIL=$PI_TEST_TAIL" \
  python3 "$REPO/tools/difftest/pty-helper.py" \
    --cols "$COLS" --rows "$ROWS" --out "$RAW" --exit-code-file "$CODE" \
    --script "$OUT/noop.json" -- "$@" >"$OUT/spawn.log" 2>&1

# 每 200 ms 采样一次原始字节数：用于区分"进程还活着但屏幕不动"与"已退出"。
: > "$OUT/liveness.txt"
elapsed=0
last=-1
while [ "$elapsed" -lt "$TIMEOUT" ]; do
  if [ -f "$CODE" ]; then
    printf 'code=%s\n' "$(cat "$CODE")" > "$OUT/exit.txt"
    break
  fi
  size=$(wc -c < "$RAW" 2>/dev/null || echo 0)
  printf 't=%s size=%s\n' "$elapsed" "$size" >> "$OUT/liveness.txt"
  last=$size
  sleep 0.2
  elapsed=$((elapsed + 1))
done
if [ -f "$CODE" ]; then
  exit 0
fi
echo "keys-run: 超时 ${TIMEOUT}s（最后字节数 $last），按进程组清理" >&2
"$REPO/tools/difftest/safe-kill.sh" --pidfile "$PIDFILE" --timeout 5 >/dev/null 2>&1 || true
exit 3
