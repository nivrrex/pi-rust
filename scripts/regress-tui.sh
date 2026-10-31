#!/bin/sh
# TUI 全场景回归（严格逐帧）：默认并行 4，**LLM 流式场景串行**。
#
# 背景（P8c/S）：`T10`（两轮 18s 固定 delay）与 `T52-tree-summarize`（/tree 摘要需要
# 第二次 LLM 调用）都靠**固定时长 + waitStable** 定位快照点；并行跑 4 个时机器负载升高，
# 快照会落到回合还没结束的帧上，footer 上下文百分比就停在 `1.1%` 而不是稳定的 `1.2%`。
# 这不是渲染差异，是**场景对负载敏感**：串行跑两场景各 20 次全部 same（见 notes/evidence）。
# 因此这里把"含 LLM 回合 + 固定 delay"的场景放进 SERIAL 列表按顺序跑，其余照旧并行。
#
# 用法：
#   scripts/regress-tui.sh [--runs N] [--jobs N] [--bin <pi-rs 路径>]
# 判据不放宽：每个场景与 `golden-tui/<名>/run-1` 严格逐帧比较。
set -u
REPO=$(cd "$(dirname "$0")/.." && pwd)
RUNS=3
JOBS=4
BIN="${TUI_CMD:-$REPO/target/debug/pi-rs}"
while [ $# -gt 0 ]; do
  case "$1" in
    --runs) RUNS=$2; shift 2 ;;
    --jobs) JOBS=$2; shift 2 ;;
    --bin) BIN=$2; shift 2 ;;
    *) echo "未知参数 $1" >&2; exit 2 ;;
  esac
done
# 对负载敏感的场景（固定 delay 等 LLM 回合）：串行。
SERIAL="T10 T52-tree-summarize"
OUT_ROOT=${OUT_ROOT:-/tmp/regress-tui}
rm -rf "$OUT_ROOT"; mkdir -p "$OUT_ROOT"

# 主题维度：`THEME=dark|light`。light 用 `scenarios-tui-light/` + `golden-tui-light/`，
# 缺省 dark 保持原有路径（`scenarios-tui/` + `golden-tui/`），因此默认行为不变。
THEME=${THEME:-dark}
case "$THEME" in
  dark) SCEN_DIR=scenarios-tui; GOLD_DIR=golden-tui ;;
  light) SCEN_DIR=scenarios-tui-light; GOLD_DIR=golden-tui-light ;;
  *) echo "regress-tui: THEME 只接受 dark|light，收到 $THEME" >&2; exit 2 ;;
esac

export THEME SCEN_DIR GOLD_DIR

run_one() {
  name=$1; run=$2; out="$OUT_ROOT/$name/run-$run"
  mkdir -p "$out"
  unshare -n sh -c "ip link set lo up; env PATH=\"$PATH\" TUI_CMD=$BIN DIFTEST_IMPL=pi-rs \
    node $REPO/tools/difftest/run-tui.mjs --scenario $REPO/$SCEN_DIR/$name.json --out $out" \
    > "$out/run.log" 2>&1
  gold="$REPO/$GOLD_DIR/$name/run-1/raw.bin"
  raw="$out/run-1/raw.bin"
  if [ ! -f "$gold" ]; then echo "$name run-$run nogolden"; return; fi
  if [ ! -f "$raw" ]; then echo "$name run-$run runfail"; return; fi
  # 每个场景可能声明自己的 `tui.cols/rows`（如 T05-w80）；比较必须用**场景的**尺寸，
  # 否则 80 列录的 golden 会被按 100 列回放，必然 different。
  dims=$(node -e 'const fs=require("fs");const s=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));const t=s.tui||{};process.stdout.write(`${t.cols||100} ${t.rows||30}`)' "$REPO/$SCEN_DIR/$name.json" 2>/dev/null)
  cols=${dims%% *}; rows=${dims##* }
  if node "$REPO/tools/difftest/compare-tui.mjs" --left "$gold" --right "$raw" \
      --cols "$cols" --rows "$rows" 2>&1 | head -1 | grep -q same; then
    echo "$name run-$run same"
  else
    echo "$name run-$run DIFFERENT"
  fi
}

serial_list=""
parallel_list=""
for path in "$REPO"/$SCEN_DIR/*.json; do
  name=$(basename "$path" .json)
  case " $SERIAL " in *" $name "*) serial_list="$serial_list $name" ;; esac
  case " $SERIAL " in *" $name "*) ;; *) parallel_list="$parallel_list $name" ;; esac
done

echo "$parallel_list" | tr ' ' '\n' | grep -v '^$' | while read -r name; do
  for run in $(seq 1 "$RUNS"); do echo "$name $run"; done
done > "$OUT_ROOT/parallel.txt"
echo "$serial_list" | tr ' ' '\n' | grep -v '^$' | while read -r name; do
  for run in $(seq 1 "$RUNS"); do echo "$name $run"; done
done > "$OUT_ROOT/serial.txt"

xargs -P "$JOBS" -n 2 sh -c "$(declare -f run_one); REPO=$REPO OUT_ROOT=$OUT_ROOT BIN=$BIN SCEN_DIR=$SCEN_DIR GOLD_DIR=$GOLD_DIR run_one \"\$0\" \"\$1\"" \
  < "$OUT_ROOT/parallel.txt" > "$OUT_ROOT/parallel.log" 2>&1
xargs -P 1 -n 2 sh -c "$(declare -f run_one); REPO=$REPO OUT_ROOT=$OUT_ROOT BIN=$BIN SCEN_DIR=$SCEN_DIR GOLD_DIR=$GOLD_DIR run_one \"\$0\" \"\$1\"" \
  < "$OUT_ROOT/serial.txt" > "$OUT_ROOT/serial.log" 2>&1

cat "$OUT_ROOT/parallel.log" "$OUT_ROOT/serial.log" > "$OUT_ROOT/result.txt"
same=$(grep -c ' same$' "$OUT_ROOT/result.txt" || true)
diff=$(grep -c ' DIFFERENT$' "$OUT_ROOT/result.txt" || true)
nogold=$(grep -c ' nogolden$' "$OUT_ROOT/result.txt" || true)
fail=$(grep -c ' runfail$' "$OUT_ROOT/result.txt" || true)
echo "same=$same different=$diff nogolden=$nogold runfail=$fail"
[ "$diff" -eq 0 ] && [ "$fail" -eq 0 ]
