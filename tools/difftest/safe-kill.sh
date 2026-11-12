#!/bin/sh
# F56/P：**安全清理**——只杀 pidfile 里三元组仍然匹配的进程树。
#
# 安全判据（缺一不可，任一不满足就**跳过**并说明原因，绝不"猜"):
#   1. pidfile 里有该 pid；
#   2. `/proc/<pid>/stat` 的 starttime 与记录一致（排除 **PID 复用**）；
#   3. `/proc/<pid>/cmdline` 的 sha256 与记录一致（排除同 pid 被换成别的程序）；
#   4. `/proc/<pid>/stat` 的 pgrp 与记录的 pgid 一致（否则不按组发信号，
#      避免误伤同组里的其它进程）。
#
# 满足时按**进程组**发 TERM（`kill -- -PGID`），等一段时间再 KILL 剩下的。
# 不接受任何"按名字/端口/模式"匹配的参数——这是 F55 误杀 openbox 的直接原因。
#
# 用法：
#   tools/difftest/safe-kill.sh --pidfile FILE [--timeout SECONDS] [--sig TERM]
set -eu

PIDFILE=""
TIMEOUT=5
SIG=TERM
while [ $# -gt 0 ]; do
  case "$1" in
    --pidfile) PIDFILE="$2"; shift 2 ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    --sig) SIG="$2"; shift 2 ;;
    *) echo "safe-kill: 未知参数 $1（本工具不接受按名字/端口/模式匹配）" >&2; exit 2 ;;
  esac
done
[ -n "$PIDFILE" ] || { echo "safe-kill: 缺 --pidfile" >&2; exit 2; }
[ -f "$PIDFILE" ] || { echo "safe-kill: pidfile 不存在：$PIDFILE（无进程可清理）"; exit 0; }

killed=0
skipped=0
while read -r pid pgid starttime expected_cmdline; do
  [ -n "${pid:-}" ] || continue
  case "$pid" in *[!0-9]*) echo "safe-kill: 跳过非法 pid '$pid'"; skipped=$((skipped+1)); continue ;; esac

  if [ ! -r "/proc/$pid/stat" ]; then
    echo "safe-kill: 跳过 $pid（进程已退出）"
    skipped=$((skipped+1))
    continue
  fi
  actual_starttime=$(awk '{print $22}' "/proc/$pid/stat")
  if [ "$actual_starttime" != "$starttime" ]; then
    echo "safe-kill: 跳过 $pid（starttime $actual_starttime ≠ $starttime ⇒ **PID 已复用**，不动它）"
    skipped=$((skipped+1))
    continue
  fi
  actual_hash=$(tr '\0' ' ' <"/proc/$pid/cmdline" | sha256sum | cut -d' ' -f1)
  if [ "$actual_hash" != "$expected_cmdline" ]; then
    echo "safe-kill: 跳过 $pid（cmdline 不符 ⇒ 不是我们起的那个进程）"
    skipped=$((skipped+1))
    continue
  fi
  actual_pgid=$(awk '{print $5}' "/proc/$pid/stat")
  if [ "$actual_pgid" != "$pgid" ]; then
    echo "safe-kill: 跳过 $pid（pgrp $actual_pgid ≠ $pgid ⇒ 不按组发信号）"
    skipped=$((skipped+1))
    continue
  fi

  # 先整组 TERM（自己起的树），再单独兜底。
  if kill -"$SIG" -- "-$pgid" 2>/dev/null; then
    echo "safe-kill: 已向进程组 -$pgid（pid $pid）发 $SIG"
  else
    kill -"$SIG" "$pid" 2>/dev/null && echo "safe-kill: 已向 pid $pid 发 $SIG（组信号失败）"
  fi
  killed=$((killed+1))
done <"$PIDFILE"

# 有界等待：等 TERM 生效，超时后对**仍然匹配三元组**的进程补 KILL。
waited=0
while [ "$waited" -lt $((TIMEOUT * 10)) ]; do
  alive=0
  while read -r pid pgid starttime expected_cmdline; do
    [ -n "${pid:-}" ] || continue
    [ -r "/proc/$pid/stat" ] || continue
    [ "$(awk '{print $22}' "/proc/$pid/stat" 2>/dev/null)" = "$starttime" ] || continue
    alive=$((alive+1))
  done <"$PIDFILE"
  [ "$alive" -eq 0 ] && break
  sleep 0.1
  waited=$((waited+1))
done

while read -r pid pgid starttime expected_cmdline; do
  [ -n "${pid:-}" ] || continue
  [ -r "/proc/$pid/stat" ] || continue
  [ "$(awk '{print $22}' "/proc/$pid/stat" 2>/dev/null)" = "$starttime" ] || continue
  [ "$(tr '\0' ' ' <"/proc/$pid/cmdline" | sha256sum | cut -d' ' -f1)" = "$expected_cmdline" ] || continue
  kill -KILL -- "-$pgid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
  echo "safe-kill: pid $pid 未响应 $SIG，已 KILL"
done <"$PIDFILE"

echo "safe-kill: 处理 $killed 个，跳过 $skipped 个"
# 清空 pidfile（保留文件本身，便于调用方复用同一个路径）
: >"$PIDFILE"
