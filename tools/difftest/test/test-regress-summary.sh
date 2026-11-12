#!/bin/sh
# F56/A4：`scripts/regress-tui.sh` 的汇总解析不得把"空清单"算成一次运行。
#
# 回归的症状：light 有 27 个场景、27 个 golden，全部 same，却报 `nogolden=1`。
# 根因：`xargs < 空文件` 仍会执行一次（`$0=sh`、`$1` 为空）⇒ `run_one sh ""` 打出
# `sh run- nogolden`。本用例直接把这条性质钉住（不跑真实场景，秒级完成）。
set -eu
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM

fail() { echo "regress-summary-test: 失败：$*" >&2; exit 1; }

# 1) 空清单：run_batch 不应产生任何输出行。
: >"$WORK/empty.txt"

# 直接从脚本里抽出 run_batch 的定义并调用（保持与生产代码同一份实现）。
{
  echo 'REPO=/nonexistent; OUT_ROOT=/nonexistent; BIN=/nonexistent'
  echo 'SCEN_DIR=x; GOLD_DIR=y; STYLE_CHECK=0'
  echo 'run_one() { echo "$0 $1 $2"; }'
  sed -n '/^run_batch() {/,/^}/p' "$REPO/scripts/regress-tui.sh"
  echo 'run_batch 1 "$1" "$2"'
} >"$WORK/harness.sh"
sh "$WORK/harness.sh" "$WORK/empty.txt" "$WORK/out.log" || fail "run_batch 在空清单上失败"
[ -s "$WORK/out.log" ] && fail "空清单不应产生任何行，实际：$(cat "$WORK/out.log")"
echo "  ok: 空清单 ⇒ 零行输出（不再出现 'sh run- nogolden'）"

# 2) 非空清单：每行恰好调用一次 run_one，$0/$1 原样传入。
printf 'T01 1\nT02 2\n' >"$WORK/two.txt"
sh "$WORK/harness.sh" "$WORK/two.txt" "$WORK/out2.log" || fail "run_batch 在非空清单上失败"
[ "$(wc -l <"$WORK/out2.log")" -eq 2 ] || fail "非空清单应产出 2 行：$(cat "$WORK/out2.log")"
grep -q 'T01 1' "$WORK/out2.log" || fail "T01 行缺失：$(cat "$WORK/out2.log")"
grep -q 'T02 2' "$WORK/out2.log" || fail "T02 行缺失：$(cat "$WORK/out2.log")"
echo "  ok: 非空清单 ⇒ 每场景每 run 恰好一行"

echo "regress-summary-test: 2 项全部通过"
