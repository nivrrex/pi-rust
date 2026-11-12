#!/bin/sh
# F55/C：把**用户这台 Linux 的真实配置 + 26 次压缩的 116 MB 会话**加入 real-parity。
#
# 与 `real-parity.sh` 的差别：
# - 语料是 `tmp/realcfg-linux`（`mk-realcfg.sh --from-home` 生成）；
# - 恢复用 `-c`（`run-real.sh --continue`），因为真机就是 `-c`
#   （ref 的 `-c` 与 `--session <file>` 是**两条不同分支**：`session-manager.ts:1779` vs `:1791-1800`）；
# - 两侧的 `--out` 目录**等长**（`a1`/`a2`）：`[Extensions]` 展开态会打印扩展入口的
#   绝对路径，80 列下它会折行；目录名长度不同会让折行位置不同，产生与行为无关的假差异
#   （实测 `ts`/`pi-rs` 差 3 字符 ⇒ 一侧折 2 行、一侧 1 行，整屏位移 2 行）。
#
# 比较口径与 `real-parity.sh` 完全一致（`compare-steps.py`，只 rstrip 行尾空白）。
set -eu
REPO=$(cd "$(dirname "$0")/../.." && pwd)
cd "$REPO"

: "${TS_PI_ENTRY:?需要 TS_PI_ENTRY=<pi-ts>/dist/bundle/cli.js}"

OUT="$REPO/tmp/real-parity-linux"
KINDS=${MATRIX_KEYS:-"none,ctrl+o"}
ROWS=40

if [ "${SKIP_REALCFG:-0}" != "1" ]; then
  "$REPO/tools/difftest/mk-realcfg.sh" --from-home >/dev/null
fi
REALCFG=${REALCFG:-$REPO/tmp/realcfg-linux}
[ -f "$REALCFG/env.sh" ] || { echo "real-parity-linux: 缺 $REALCFG/env.sh" >&2; exit 1; }
# shellcheck disable=SC1091
. "$REALCFG/env.sh"

rm -rf "$OUT"
mkdir -p "$OUT"
FAILED=0
SUMMARY="$OUT/summary.txt"
: > "$SUMMARY"

for cols in 110 80; do
  for config in ext ne; do
    for keys in $(printf '%s' "$KINDS" | tr ',' ' '); do
      [ "$keys" = "none" ] && keys=""
      label="cols${cols}-${config}-keys$(printf '%s' "$keys" | tr ',' '_' | sed 's/^$/none/')"
      cell="$OUT/$label"
      rm -rf "$cell"
      # 等长的 out 目录名（见文件头注释）。
      for pair in "ts a1" "pi-rs a2"; do
        impl=${pair%% *}
        short=${pair##* }
        REALCFG="$REALCFG" "$REPO/tools/difftest/run-real.sh" --impl "$impl" --keys "$keys" \
          --cols "$cols" --rows "$ROWS" --config "$config" --continue \
          --out "$cell/$short" >"$cell.$short.log" 2>&1 || true
      done
      echo "=== $label" | tee -a "$SUMMARY"
      # 语料归一化（**不改比较器**，只换掉"目录名"这一层）：
      # - 两侧的 agent 目录必然不同（`a1/agent` vs `a2/agent`）；
      # - 语料根目录本身也出现在 `[Extensions]` 展开行里。
      # 这属**路径名**差异而非行为差异（ref 打印的就是它自己的 agent 目录）。
      for short in a1 a2; do
        for file in "$cell/$short"/steps/step-*.txt; do
          [ -f "$file" ] || continue
          sed -i -e "s#$cell/$short/agent#<AGENT>#g" -e "s#$cell#<CELL>#g" "$file"
        done
      done
      if ! python3 "$REPO/tools/difftest/compare-steps.py" \
          --left "$cell/a1/steps" --right "$cell/a2/steps" \
          --label "$label" >>"$SUMMARY" 2>&1; then
        FAILED=$((FAILED + 1))
      fi
    done
  done
done

echo "real-parity-linux: 失败格子 $FAILED（明细见 $SUMMARY）"
[ "$FAILED" -eq 0 ]
