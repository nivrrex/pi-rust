#!/bin/sh
# CLI 场景的批量差分回归（对应 `golden/` 那批，94 个场景）。
#
# 为什么需要：TUI 侧早有 `scripts/regress-tui.sh` 做全量回归，而 CLI 侧**没有**——
# 94 个 golden 只有 `check-s01-pin.sh` 在 precommit 里钉住 S01 一个，
# 其余 93 个只能手工 `run.mjs` + `compare.mjs` 逐个跑。F123 补上。
#
# **必须设 `TS_PI_ENTRY` 与 `PI_TS_PACKAGE_DIR`**：后者决定 system 提示里 docs 段的
# 路径，缺了会出现"差 213 字符"的假回退（`check-s01-pin.sh` 的头部注释有完整说明）。
#
# 用法：
#   export PI_TS_PACKAGE_DIR=/path/to/@earendil-works/pi-coding-agent
#   sh scripts/regress-cli.sh [场景名…]      # 不给参数 = 全部有 golden 的场景
set -eu

[ -n "${PI_TS_PACKAGE_DIR:-}" ] || {
  echo "regress-cli: 需要 PI_TS_PACKAGE_DIR（pnpm 的 pi-coding-agent 包目录）" >&2
  echo "  缺它会因 docs 段路径不同产生假回退（差 213 字符）。" >&2
  exit 2
}
REPO=$(cd "$(dirname "$0")/.." && pwd)
export TS_PI_ENTRY="${TS_PI_ENTRY:-$PI_TS_PACKAGE_DIR/dist/bundle/cli.js}"
export PI_TS_PACKAGE_DIR
export DIFTEST_IMPL=pi-rs
# `tools/difftest/run.mjs` **拒绝绝对路径的 `--out`**（它会把产出 join 进仓库，
# 怕写回 `golden/`）。所以产出放在仓库内的 `tmp/` 下，跑完由退出钩子清掉。
# 与 `regress-tui.sh` 同理：bash 报错语言随 `LANGUAGE` 变，golden 是中文录的。
export LANGUAGE="${DIFTEST_LANGUAGE:-zh_CN}"
export LANG="${DIFTEST_LOCALE:-zh_CN.UTF-8}"

OUT_ROOT="$REPO/tmp/regress-cli"
JOBS=${JOBS:-4}

rm -rf "$OUT_ROOT"
mkdir -p "$OUT_ROOT"
trap 'rm -rf "$OUT_ROOT"' EXIT INT TERM

if [ "$#" -gt 0 ]; then
  names="$*"
else
  names=""
  for gold in "$REPO"/golden/*/; do
    name=$(basename "$gold")
    [ -f "$REPO/scenarios/$name.json" ] && names="$names $name"
  done
fi

run_one() {
  name=$1
  out="$OUT_ROOT/$name"
  mkdir -p "$out"
  # `--out` 必须是**相对仓库根**的路径（`run.mjs` 拒绝任何绝对路径）。
  rel="tmp/regress-cli/$name"
  if ! ( cd "$REPO" && timeout 900 node tools/difftest/run.mjs "scenarios/$name.json" --out "$rel" ) \
      > "$out/run.log" 2>&1; then
    echo "$name RUNFAIL"
    return
  fi
  # 与 `run.mjs` 同理：`compare.mjs` 也拒绝绝对路径的 `--out`。
  result=$( cd "$REPO" && node tools/difftest/compare.mjs "$name" --out "$rel" 2>&1 | head -1)
  case "$result" in
    *"A=same B=same C=same"*) echo "$name same" ;;
    *) echo "$name DIFFERENT" ;;
  esac
}

# 每个场景要起 mock + pty，并行度过高会互相拖慢；默认 4。
#
# `export -f` 在 POSIX sh 下不生效，所以把 `run_one` 的函数体作为**独立脚本**
# 落到 `$OUT_ROOT`，子进程直接执行它——这样不依赖 bash 的函数导出。
{
  echo '#!/bin/sh'
  sed -n '/^run_one() {/,/^}/p' "$REPO/scripts/regress-cli.sh"
  echo 'run_one "$1"'
} > "$OUT_ROOT/run-one.sh"
chmod +x "$OUT_ROOT/run-one.sh"
export REPO OUT_ROOT
# 子进程走 `xargs sh -c`，环境必须显式导出（`PI_JITI` 由调用方提供时一并带过）。
[ -n "${PI_JITI:-}" ] && export PI_JITI
true

printf '%s\n' $names | grep -v '^$' | \
  xargs -r -P "$JOBS" -I{} sh "$OUT_ROOT/run-one.sh" {} | tee "$OUT_ROOT/result.txt"

same=$(grep -c ' same$' "$OUT_ROOT/result.txt" || true)
diff=$(grep -c ' DIFFERENT$' "$OUT_ROOT/result.txt" || true)
fail=$(grep -c ' RUNFAIL$' "$OUT_ROOT/result.txt" || true)
total=$((same + diff + fail))
printf '\nsame=%s different=%s runfail=%s（共 %s）\n' "$same" "$diff" "$fail" "$total"
[ "$diff" -eq 0 ] && [ "$fail" -eq 0 ]
