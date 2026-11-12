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
# TS 侧"就绪"判据沿用启动屏文案。**已知限制**：恢复大会话时 TS 的启动是多帧序列，
# 用更晚的标记或更长的静止窗口都会拍到中间态（实测三种口径都在两个 cell 上给出
# 相反结果），因此该矩阵的 TS 侧存在时序不确定性，见 notes/p9-f53/plan.md。
WAIT_CONTAINS='Pi can explain' 
KEYS=""
COLS=""
ROWS=40
CONFIG=""
KEEP=0
MATRIX=1
CONTINUE_MODE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --session) SESSION_KIND="$2"; MATRIX=0; shift 2 ;;
    --keys) KEYS="$2"; MATRIX=0; shift 2 ;;
    --cols) COLS="$2"; shift 2 ;;
    --rows) ROWS="$2"; shift 2 ;;
    --config) CONFIG="$2"; MATRIX=0; shift 2 ;;
    --out) OUT_ROOT="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    # F55/C：`--continue` ⇒ 用 `-c` 而不是 `--session <file>`。
    # 真实 Linux 会话 116 MB / 64196 条目，`--session` 与 `-c` 在 ref 里走**不同**的
    # 恢复分支（`session-manager.ts:1779` vs `:1791-1800`），`-c` 才是真机用的那条。
    --continue) CONTINUE_MODE=1; shift ;;
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
    if [ "$CONTINUE_MODE" = "1" ]; then
      "$REPO/tools/difftest/run-real.sh" --impl "$impl" --keys "$keys" --cols "$cols" --rows "$ROWS" \
        --config "$config" --continue --out "$cell/$impl" >"$cell.log" 2>&1 || true
    else
      "$REPO/tools/difftest/run-real.sh" --impl "$impl" --keys "$keys" --cols "$cols" --rows "$ROWS" \
        --config "$config" --session "$SESSION_FILE" --out "$cell/$impl" >"$cell.log" 2>&1 || true
    fi
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

# F54/Z3：**窗口缩放**格（`pty-resize.py` + SIGWINCH）。与按键格并列：
# 同一 realcfg、同一会话，脚本里按序注入长文本 → 110→60→110 两次缩放 → 退出，
# 每个 mark 按**当时的宽度**还原屏幕后逐步逐行精确比较。
run_resize_cell() {
  label="$1"
  config="$2"
  cell="$OUT_ROOT/$label"
  rm -rf "$cell"
  script="$OUT_ROOT/$label.script.json"
  cat > "$script" <<'JSON'
[
  {"waitFor": {"contains": "Pi can explain", "timeoutMs": 60000}},
  {"waitStable": {"quietMs": 2500, "timeoutMs": 180000}},
  {"mark": "wide"},
  {"text": "缩放重排测试一二三四五六"},
  {"waitStable": {"quietMs": 1200, "timeoutMs": 60000}},
  {"mark": "typed"},
  {"resize": {"cols": 60, "rows": 30}},
  {"waitStable": {"quietMs": 1500, "timeoutMs": 60000}},
  {"mark": "narrow"},
  {"resize": {"cols": 110, "rows": 30}},
  {"waitStable": {"quietMs": 1500, "timeoutMs": 60000}},
  {"mark": "back"},
  {"text": "\u0004"}
]
JSON
  for impl in ts pi-rs; do
    "$REPO/tools/difftest/run-resize.sh" --impl "$impl" --script "$script" --cols 110 --rows 30 \
      --config "$config" --session "$SESSION_FILE" --out "$cell/$impl" >"$cell.log" 2>&1 || true
  done
  echo "=== $label" | tee -a "$SUMMARY"
  if ! python3 "$REPO/tools/difftest/compare-steps.py" \
      --left "$cell/ts/steps" --right "$cell/pi-rs/steps" --steps 3 \
      --label "$label" >>"$SUMMARY" 2>&1; then
    FAILED=$((FAILED + 1))
  fi
}

# 矩阵规模：默认 24 格（6 个键序列）；`MATRIX_KEYS` 可缩小（真实 Linux 会话 116 MB，
# 每格 TS 侧要几十秒，CI 里跑全 24 格不现实）。空格分隔，空串用 `none` 表示。
# **默认保持 6 个（24 格）**：既有判据不得放宽。真实 Linux 语料那套自己传更小的集合。
MATRIX_KEYS=${MATRIX_KEYS:-"none,ctrl+o,ctrl+o_ctrl+o,ctrl+t,ctrl+t_ctrl+t,ctrl+o_ctrl+t"}
if [ "$MATRIX" -eq 1 ]; then
  for cols in 110 80; do
    for config in ext ne; do
      for keys in $(printf '%s' "$MATRIX_KEYS" | tr ',' ' '); do
        [ "$keys" = "none" ] && keys=""
        run_cell "$cols" "$config" "$keys"
      done
    done
  done
  if [ "${MATRIX_RESIZE:-1}" = "1" ]; then
    for config in ext ne; do
      run_resize_cell "resize-${config}" "$config"
    done
  fi
else
  run_cell "${COLS:-110}" "${CONFIG:-ext}" "$KEYS"
fi

if [ "$KEEP" -eq 0 ]; then
  : # 保留录制，供人查看首个差异行；收口时随 tmp/ 一起删
fi

echo "real-parity: 失败格子 $FAILED（明细见 $SUMMARY）"
[ "$FAILED" -eq 0 ]
