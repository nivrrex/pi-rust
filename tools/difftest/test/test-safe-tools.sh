#!/bin/sh
# F56/P：`safe-spawn.sh` / `safe-kill.sh` 的行为用例（POSIX sh，随 precommit 跑）。
#
# 三条断言对应 F55 误杀 openbox 的三种失效方式：
# 1. **PID 复用**：pidfile 里的 starttime 与当前不符 ⇒ 必须拒绝；
# 2. **cmdline 不符**：同一 pid 已被换成别的程序 ⇒ 必须拒绝；
# 3. **正常清理**：三元组匹配 ⇒ 按进程组 TERM 掉，且不留活口。
# 另有第 4 条：拒绝任何"按名字/端口/模式"的参数。
set -eu

REPO=$(cd "$(dirname "$0")/../../.." && pwd)
SPAWN="$REPO/tools/difftest/safe-spawn.sh"
KILL="$REPO/tools/difftest/safe-kill.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM

fail() { echo "safe-tools-test: 失败：$*" >&2; exit 1; }

# ---------------------------------------------------------------- 1) 正常清理
pid=$("$SPAWN" --pidfile "$WORK/pf1" --out "$WORK/l1.log" -- sleep 300)
[ -n "$pid" ] || fail "safe-spawn 未返回 pid"
[ "$(wc -l <"$WORK/pf1")" -eq 1 ] || fail "pidfile 应有一条记录"
"$KILL" --pidfile "$WORK/pf1" --timeout 3 >"$WORK/k1.log" 2>&1 || fail "safe-kill 退出码非 0"
if [ -r "/proc/$pid/stat" ]; then
  # 允许极短的退出延迟，最多再等 1s
  i=0
  while [ "$i" -lt 10 ] && [ -r "/proc/$pid/stat" ]; do sleep 0.1; i=$((i+1)); done
fi
[ ! -r "/proc/$pid/stat" ] || fail "正常清理后 $pid 仍存活"
grep -q "处理 1 个" "$WORK/k1.log" || fail "safe-kill 应报告处理 1 个：$(cat "$WORK/k1.log")"
echo "  ok: 正常清理（按进程组 TERM）"

# ------------------------------------------------- 2) PID 复用（starttime 不符）
me=$$
fake_start=$(( $(awk '{print $22}' "/proc/$me/stat") - 5000 ))
fake_hash=$(printf 'definitely-not-this-process' | sha256sum | cut -d' ' -f1)
printf '%s %s %s %s\n' "$me" "$(awk '{print $5}' "/proc/$me/stat")" "$fake_start" "$fake_hash" >"$WORK/pf2"
"$KILL" --pidfile "$WORK/pf2" --timeout 1 >"$WORK/k2.log" 2>&1 || true
grep -q "PID 已复用" "$WORK/k2.log" || fail "starttime 不符时应报 'PID 已复用'：$(cat "$WORK/k2.log")"
kill -0 "$me" 2>/dev/null || fail "starttime 不符时误杀了测试 shell"
echo "  ok: starttime 不符 ⇒ 拒绝（PID 复用保护）"

# --------------------------------------------------- 3) cmdline 不符（同 pid 换程序）
real_start=$(awk '{print $22}' "/proc/$me/stat")
printf '%s %s %s %s\n' "$me" "$(awk '{print $5}' "/proc/$me/stat")" "$real_start" "$fake_hash" >"$WORK/pf3"
"$KILL" --pidfile "$WORK/pf3" --timeout 1 >"$WORK/k3.log" 2>&1 || true
grep -q "cmdline 不符" "$WORK/k3.log" || fail "cmdline 不符时应报 'cmdline 不符'：$(cat "$WORK/k3.log")"
kill -0 "$me" 2>/dev/null || fail "cmdline 不符时误杀了测试 shell"
echo "  ok: cmdline 不符 ⇒ 拒绝"

# ------------------------------------------------------- 4) 拒绝按名/端口/模式匹配
if "$KILL" --pidfile "$WORK/pf1" --pattern openbox >"$WORK/k4.log" 2>&1; then
  fail "safe-kill 不应接受 --pattern"
fi
grep -q "不接受按名字" "$WORK/k4.log" || fail "拒绝 --pattern 时应给出明确原因"
echo "  ok: 拒绝按名字/端口/模式匹配"

echo "safe-tools-test: 4 项全部通过"
