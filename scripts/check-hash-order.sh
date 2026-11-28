#!/bin/sh
# 输出路径上禁止用 `HashMap`/`HashSet` 的**迭代顺序**决定结果。
#
# 为什么：Rust 的 `HashMap`/`HashSet` 每个进程用**随机种子**，迭代顺序每次启动都不同
# （比 `read_dir` 更危险——后者至少在单次运行内稳定）。一旦某处拿它定序，
# 表现就是"偶发、不可复现、换个进程就变"，对拍会随机红、`/resume` 的默认选中项也会跳。
#
# 判据（只看**可能影响输出**的形状）：
#   `for … in <expr>.iter()/.keys()/.values()` 且 `<expr>` 名字里带 map/set/by_id/kids/labels
# 允许：`BTreeMap`/`BTreeSet`（有序）、`Vec`、`IndexMap`。
# 例外可用 `// hash-order-ok: <理由>` 就地豁免。
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO"

PATTERN='for[[:space:]].*[[:space:]]in[[:space:]]\+[A-Za-z_][A-Za-z0-9_]*\.\(iter()\|keys()\|values()\)'
SUSPECT='\(map\|set\|by_id\|kids\|labels\|scores\|counts\)\.'

found=0
checked=0
for path in crates/*/src/*.rs crates/*/src/**/*.rs; do
  [ -f "$path" ] || continue
  checked=$((checked + 1))
  hits=$(grep -n "$PATTERN" "$path" 2>/dev/null | grep -i "$SUSPECT" || true)
  [ -n "$hits" ] || continue
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    lineno=${line%%:*}
    # 就地豁免：该行或上一行有 hash-order-ok。
    body=$(sed -n "${lineno}p" "$path")
    case "$body" in *"hash-order-ok"*) continue ;; esac
    prev=$(sed -n "$((lineno > 1 ? lineno - 1 : 1))p" "$path")
    case "$prev" in *"hash-order-ok"*) continue ;; esac
    printf '%s:%s: %s\n' "$path" "$lineno" "$(printf '%s' "$body" | sed 's/^[[:space:]]*//')"
    found=$((found + 1))
  done <<EOF2
$hits
EOF2
done

printf 'check-hash-order: 检查 %d 个文件\n' "$checked"
if [ "$found" -gt 0 ]; then
  printf 'check-hash-order: FAILED（%d 处可能按哈希顺序定序；改用 BTreeMap 或显式排序，或加 `// hash-order-ok: 理由`）\n' "$found"
  exit 1
fi
printf 'check-hash-order: OK\n'
