#!/bin/sh
# F54/E：样式层对拍（**诊断工具，不作闸门**）。
#
# 用同一份 realcfg 真实会话，跑一组代表性配置（110 列 dark、80 列 dark、110 列 light），
# 每一步按 `marks.jsonl` 截断 raw，再逐单元格比较 SGR（前景/背景/粗/斜/下划线/暗）。
# 差异**按行类别汇总**（工具头、diff 增删、markdown、页脚、todo、思考行…），每类给首个
# 最小复现位置；是否进验收由人决定 —— 本脚本恒以退出码 0 结束。
#
# 用法：tools/difftest/real-style.sh [--keys "ctrl+o"] [--config ext|ne] [--out DIR]
set -eu
REPO=$(cd "$(dirname "$0")/../.." && pwd)
cd "$REPO"

KEYS=""
CONFIG=ext
OUT=${OUT_DIR:-$REPO/tmp/real-style}
while [ $# -gt 0 ]; do
  case "$1" in
    --keys) KEYS="$2"; shift 2 ;;
    --config) CONFIG="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    *) echo "real-style: 未知参数 $1" >&2; exit 2 ;;
  esac
done

"$REPO/tools/difftest/mk-realcfg.sh" >/dev/null
# shellcheck disable=SC1091
. "$REPO/tmp/realcfg/env.sh"
rm -rf "$OUT"
mkdir -p "$OUT"

run() {
  label="$1"; cols="$2"; theme="$3"
  cell="$OUT/$label"
  mkdir -p "$cell"
  for impl in ts pi-rs; do
    "$REPO/tools/difftest/run-real.sh" \
      --impl "$impl" --keys "$KEYS" --cols "$cols" --rows 40 --config "$CONFIG" \
      --session "$REALCFG_SESSION_NOTHk" --out "$cell/$impl" >"$cell/$impl.log" 2>&1 || true
  done
  echo "=== $label (${cols}x40, theme=$theme)"
  node "$REPO/tools/difftest/real-style.mjs" \
    --left "$cell/ts" --right "$cell/pi-rs" --cols "$cols" --rows 40 --steps 1
}

# 主题由 realcfg 的 settings 决定（同一份副本，两侧同源）；`theme` 只是标签，
# 想跑 light 时用 `REALCFG=<light 副本> tools/difftest/real-style.sh`。
run dark110 110 dark
run dark80 80 dark
echo "real-style: 诊断完成（本脚本不作闸门，退出码恒为 0）"
exit 0
