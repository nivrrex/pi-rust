#!/bin/sh
# TUI 全场景回归（严格逐帧）：默认并行 4，**LLM 流式场景串行**。
#
# 背景（P9-F5 更正）：`T10`（两轮 18s 固定 delay）与 `T52-tree-summarize`（/tree 摘要）曾被记为
# "对负载敏感、footer 停在 1.1%"。**实测否定**：各连跑 30 次结果完全一致（不是偶发），
# 真正的变量是 **`TS_PI_ENTRY`（→ `PI_PACKAGE_DIR`）有没有设置**——它决定 system prompt 的 docs 段，
# 少 213 字符 ⇒ 上下文百分比从 `1.2%` 掉到 `1.1%`（见 `notes/p9-f5/flaky.md`）。
# 设置后 `--jobs 4` 连跑两次均为 80 same / 0 different。故本脚本现在**缺该变量直接报错退出**，
# 不再产出"看起来像偶发差异"的结果；SERIAL 列表保留（仍可减少固定 delay 场景的排队抖动）。
#
# 用法：
#   scripts/regress-tui.sh [--runs N] [--jobs N] [--bin <pi-rs 路径>]
# 判据不放宽：每个场景与 `golden-tui/<名>/run-1` 严格逐帧比较。
set -u
# 缺 `TS_PI_ENTRY`（或 `PI_TS_PACKAGE_DIR`）时，system prompt 的 docs 段会少 213 字符，
# T10/T52 的 footer 上下文百分比因此差 0.1pp——与渲染无关，纯属环境缺失。直接失败，避免误判为偶发。
# F30/P1：golden 语料是**两类录制路径**混着的 ⇒ 按场景名选包路径。
#   PI_TS_SHORT：<短路径包>/dist/bundle/cli.js 这类**短路径**（多数 TUI golden）
#   PI_TS_LONG ：pnpm 全局包一类的**长路径**（T10 / T52-tree-summarize 与 print 层 S01）
# 两个变量都必须由调用方给出；脚本里不写死任何用户路径。
if [ -z "${PI_TS_SHORT:-}" ] || [ -z "${PI_TS_LONG:-}" ]; then
  echo "regress-tui: 需要 PI_TS_SHORT 与 PI_TS_LONG（golden 语料分两类录制路径；缺一个就会得到 0.1pp 假差异）" >&2
  echo "  export PI_TS_SHORT=/path/pi-ts-0.87.1/dist/bundle/cli.js" >&2
  echo "  export PI_TS_LONG=/path/pnpm/.../pi-coding-agent/dist/bundle/cli.js" >&2
  exit 2
fi
# 用长路径录制的场景（其余一律短路径）。
LONG_PATH_SCENARIOS=" T10 T52-tree-summarize "
# 必须 export：实际的 run_one 跑在 `xargs sh -c "$(declare -f run_one)…"` 子进程里，
# 不 export 的话子进程看不到这几个变量（表现为 TS_PI_ENTRY 为空 ⇒ docs 段退回短版本）。
export PI_TS_SHORT PI_TS_LONG LONG_PATH_SCENARIOS
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
  case "$LONG_PATH_SCENARIOS" in
    *" $name "*) ts_entry=$PI_TS_LONG ;;
    *) ts_entry=$PI_TS_SHORT ;;
  esac
  unshare -n sh -c "ip link set lo up; env PATH=\"$PATH\" TS_PI_ENTRY=$ts_entry TUI_CMD=$BIN DIFTEST_IMPL=pi-rs \
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
