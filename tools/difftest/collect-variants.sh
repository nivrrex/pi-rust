#!/usr/bin/env bash
# 收集一个场景的 A 层形态集合（A4）。
#
# 每次运行都用**独立调用**，避免同一次调用内的相互影响；随后对 A 层产物
# （session.jsonl + fs.json + 退出码）归一化去重，每种形态存成
# golden/<场景>/variants/vN/，并把出现次数写进 meta.json 的 variants 字段。
#
# 用法：tools/difftest/collect-variants.sh <场景号> [次数，默认 10]

set -euo pipefail
cd "$(dirname "$0")/../.."

# node 由 fnm 管理，PATH 里未必带 fnm 的 bin 目录（run.sh 里已有同样的兜底）。
if ! command -v node >/dev/null 2>&1; then
  for d in "$HOME"/.local/share/fnm/node-versions/*/installation/bin; do
    [ -x "$d/node" ] && PATH="$d:$PATH" && break
  done
fi

id="${1:?需要场景号}"
runs="${2:-10}"
work="golden/_collect/$id"
rm -rf "$work"
mkdir -p "$work"

for i in $(seq 1 "$runs"); do
  ./tools/difftest/run.sh "scenarios/$id.json" --runs 1 --out "$work/r$i" >/dev/null 2>&1 || true
done

node tools/difftest/dedupe-variants.mjs "$id" "$work" "$runs"
