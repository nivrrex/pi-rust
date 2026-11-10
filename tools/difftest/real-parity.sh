#!/bin/sh
# F53/T4：`real-parity` —— 同一 realcfg、同一会话、同一宽度、同一按键序列下，
# 逐步比较 TS 与 pi-rs 的屏幕文本（`step-N.txt`），并对两侧各跑 T5 不变量断言。
#
# **不替代 regress**：regress 用合成夹具比 golden（字节级、含颜色），这里用**真实配置**
# 比"人眼看得见的东西"（去 ANSI 的屏幕文本 + 状态快照），专治只有真机才出现的现象。
#
# 用法：
#   tools/difftest/real-parity.sh [--session real|nothk] [--keys "ctrl+o,ctrl+o"]
#       [--cols 110] [--rows 40] [--config ext|ne] [--out DIR] [--keep]
#
# 不带参数时跑**验收矩阵**：宽度 110/80 × 配置 ext/ne × 按键序列 6 种 × 会话 nothk。
set -eu

# TS 侧入口必须由调用方给出（脚本不写死任何本机路径；缺了就是"假回退"）。
: "${TS_PI_ENTRY:?需要 TS_PI_ENTRY=<pi-ts>/dist/bundle/cli.js}"
export TS_PI_ENTRY

REPO=$(cd "$(dirname "$0")/../.." && pwd)
OUT_ROOT="$REPO/tmp/real-parity"
SESSION_KIND=nothk
KEYS=""
COLS=""
ROWS=40
CONFIG=""
KEEP=0
MATRIX=1

while [ $# -gt 0 ]; do
  case "$1" in
    --session) SESSION_KIND="$2"; MATRIX=0; shift 2 ;;
    --keys) KEYS="$2"; MATRIX=0; shift 2 ;;
    --cols) COLS="$2"; shift 2 ;;
    --rows) ROWS="$2"; shift 2 ;;
    --config) CONFIG="$2"; MATRIX=0; shift 2 ;;
    --out) OUT_ROOT="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    *) echo "real-parity: 未知参数 $1" >&2; exit 2 ;;
  esac
done

REALCFG=${REALCFG:-$REPO/tmp/realcfg}
[ -f "$REALCFG/env.sh" ] || { echo "real-parity: 先跑 tools/difftest/mk-realcfg.sh" >&2; exit 1; }
# shellcheck disable=SC1091
. "$REALCFG/env.sh"
case "$SESSION_KIND" in
  real) SESSION_FILE="$REALCFG_SESSION_FILE" ;;
  nothk) SESSION_FILE="$REALCFG_SESSION_NOTHk" ;;
  *) echo "real-parity: --session 只接受 real|nothk" >&2; exit 2 ;;
esac

mkdir -p "$OUT_ROOT"
FAILED=0
SUMMARY="$OUT_ROOT/summary.txt"
: > "$SUMMARY"

run_cell() {
  cols="$1"
  config="$2"
  keys="$3"
  label="cols${cols}-${config}-keys$(printf '%s' "$keys" | tr ',' '_' | sed 's/^$/none/')"
  cell="$OUT_ROOT/$label"
  rm -rf "$cell"
  for impl in ts pi-rs; do
    "$REPO/tools/difftest/run-real.sh" --impl "$impl" --keys "$keys" --cols "$cols" --rows "$ROWS" \
      --config "$config" --session "$SESSION_FILE" --out "$cell/$impl" >"$cell.log" 2>&1 || true
  done
  # 比较步数 = 按键数 + 1（初态）；退出那一步（ctrl+d 之后）不参与比较。
  key_count=$(cat "$cell/pi-rs/key_count" 2>/dev/null || echo 0)
  echo "=== $label" | tee -a "$SUMMARY"
  if ! python3 "$REPO/tools/difftest/compare-steps.py" \
      --left "$cell/ts/steps" --right "$cell/pi-rs/steps" --steps "$key_count" \
      --label "$label" >>"$SUMMARY" 2>&1; then
    FAILED=$((FAILED + 1))
  fi
  # T5：两侧各跑不变量断言（宽度 / 控制字符 / 页脚 / 内容保全）。
  for impl in ts pi-rs; do
    set +e
    python3 "$REPO/tools/difftest/assert-steps.py" --dir "$cell/$impl/steps" --cols "$cols" \
      --expect-branch "$REALCFG_BRANCH" --expect-thinking high --steps "$key_count" \
      >>"$SUMMARY" 2>&1
    status=$?
    set -e
    if [ "$status" -ne 0 ]; then
      FAILED=$((FAILED + 1))
    fi
  done
}

if [ "$MATRIX" -eq 1 ]; then
  for cols in 110 80; do
    for config in ext ne; do
      for keys in "" "ctrl+o" "ctrl+o,ctrl+o" "ctrl+t" "ctrl+t,ctrl+t" "ctrl+o,ctrl+t"; do
        run_cell "$cols" "$config" "$keys"
      done
    done
  done
else
  run_cell "${COLS:-110}" "${CONFIG:-ext}" "$KEYS"
fi

if [ "$KEEP" -eq 0 ]; then
  : # 保留录制，供人查看首个差异行；收口时随 tmp/ 一起删
fi

echo "real-parity: 失败格子 $FAILED（明细见 $SUMMARY）"
[ "$FAILED" -eq 0 ]
