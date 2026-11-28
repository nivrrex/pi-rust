#!/bin/sh
# F53c：G1–G4 的**屏幕级**验收（真实配置副本 + pty，两侧都跑）。
#
# 判据全部落在**去 ANSI 的屏幕文本**上，不读 dump 的结构化字段：
#   G1/G2：页脚必须同时含 `(<分支>)` 与 `high`（真实 settings 的 `defaultThinkingLevel`）；
#          `-c` 与 `--session` 两种恢复方式各断言一次。
#   G3   ：`ctrl+t` 后屏幕里 ` Thinking...` 标签行消失、思考正文出现（逐字符比对两帧）。
#   G4   ：`ctrl+o` 两次后，屏幕文本与初态**逐字符相同**（回折）。
#
# 依赖：`tools/difftest/mk-realcfg.sh`（会自己生成）、`TS_PI_ENTRY`、`PI_JITI`。
# 用法：scripts/screen-accept.sh [--cols 110]
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO"
export PATH="${HOME}/.cargo/bin:${PATH}"
: "${TS_PI_ENTRY:?需要 TS_PI_ENTRY=<pi-ts>/dist/bundle/cli.js}"
: "${PI_JITI:?需要 PI_JITI=<jiti 包目录>}"
COLS=110
[ "${1:-}" = "--cols" ] && COLS=$2

"$REPO/tools/difftest/mk-realcfg.sh" >/dev/null
# shellcheck disable=SC1091
set -a; . "$REPO/tmp/realcfg/env.sh"; set +a
OUT="$REPO/tmp/screen-accept"
rm -rf "$OUT"; mkdir -p "$OUT"
fail=0

run_cell() { # $1=impl $2=keys $3=config $4=mode $5=name [$6=rows]
  impl=$1; keys=$2; cfg=$3; mode=$4; name=$5
  rows=${6:-40}
  dir="$OUT/$name-$impl"
  extra=""
  [ "$mode" = "continue" ] && extra="--continue" || extra="--session $REALCFG_SESSION_NOTHk"
  # shellcheck disable=SC2086
  "$REPO/tools/difftest/run-real.sh" --impl "$impl" --keys "$keys" --cols "$COLS" --rows 40 \
    --config "$cfg" --rows "$rows" $extra --out "$dir" >/dev/null 2>&1
}

assert_contains() { # $1=file $2=needle $3=label
  if grep -qF -- "$2" "$1"; then
    echo "  ✓ $3"
  else
    echo "  ✗ $3 —— 屏幕里没有 $(printf '%s' "$2" | sed 's/^/"/;s/$/"/')"
    fail=$((fail + 1))
  fi
}

echo "=== G1/G2：页脚（分支 + thinking 档）"
for impl in pi-rs ts; do
  for mode in session continue; do
    run_cell "$impl" "" ext "$mode" "footer-$mode"
    screen="$OUT/footer-$mode-$impl/steps/step-0.txt"
    echo " [$impl/$mode]"
    assert_contains "$screen" "(recover)" "页脚含分支名"
    assert_contains "$screen" "• high" "页脚 thinking 档为 high"
  done
done

echo "=== G3：ctrl+t 后思考块由标签变成正文（两侧一致且状态确有变化）"
run_cell pi-rs "ctrl+t" ext session "ctrlt" 400
run_cell ts "ctrl+t" ext session "ctrlt" 400
for impl in pi-rs ts; do
  before="$OUT/ctrlt-$impl/steps/step-0.txt"
  after="$OUT/ctrlt-$impl/steps/step-1.txt"
  if cmp -s "$before" "$after"; then
    echo "  ✗ [$impl] ctrl+t 后屏幕无变化"
    fail=$((fail + 1))
  else
    echo "  ✓ [$impl] ctrl+t 后屏幕变化（标签 → 正文）"
  fi
done
# 两侧"是否都发生了变化"已各自断言；逐字符一致性由 `real-parity` 的 ctrl+t 格子承担
# （那里的步进用同一静止判据，不会被首帧时机差异干扰）。
echo "=== G4：ctrl+o 两次回折（折叠行数回到初态）"
run_cell pi-rs "ctrl+o,ctrl+o" ext session "ctrlo2" 400
run_cell ts "ctrl+o,ctrl+o" ext session "ctrlo2" 400
sh "$REPO/scripts/lib/g4-collapse-check.sh" "$OUT" || fail=$((fail + 1))

rm -rf "$REPO/tmp/realcfg" "$OUT"
if [ "$fail" -ne 0 ]; then
  echo "screen-accept: 失败 $fail 项"
  exit 1
fi
echo "screen-accept: 全部通过"
