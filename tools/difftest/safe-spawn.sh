#!/bin/sh
# F56/P：**安全启动**测试进程（自建进程组 + pidfile 身份三元组）。
#
# 为什么需要：F55 期间我用 PID 清理测试进程，误杀了用户桌面进程（openbox）。
# 根因有两层：
#   1. 先记 PID、稍后再 kill —— 中间进程可能已退出、PID 被**复用**给别的进程
#      （Linux 的 pid 是循环分配的，`/proc/sys/kernel/pid_max` 默认 4194304，
#       桌面会话跑一整天后 pid 会回绕）。
#   2. 清理用 `pgrep -f <模式>` 按名称批量匹配 —— 模式一旦过宽就会命中无关进程。
#
# 本工具的做法：启动时把「pid + /proc/<pid>/stat 的 starttime + cmdline 的 sha256」
# 三元组写进 pidfile；`safe-kill.sh` 只在三元组**全部**仍然匹配时才动手，
# 并按**进程组**（`kill -- -PGID`）发信号，把子进程一起收掉。
#
# 用法：
#   tools/difftest/safe-spawn.sh --pidfile FILE --out LOG [--] CMD [ARGS…]
#
# 退出码：0 表示已成功启动（不等待结束）；被测进程的 stdout/stderr 全部进 LOG。
# 需要等待的调用方自己轮询 pidfile 里的 pid（或等 LOG 出现标记）。
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)

PIDFILE=""
LOG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --pidfile) PIDFILE="$2"; shift 2 ;;
    --out) LOG="$2"; shift 2 ;;
    --) shift; break ;;
    *) break ;;
  esac
done
[ -n "$PIDFILE" ] || { echo "safe-spawn: 缺 --pidfile" >&2; exit 2; }
[ -n "$LOG" ] || { echo "safe-spawn: 缺 --out" >&2; exit 2; }
[ $# -gt 0 ] || { echo "safe-spawn: 缺命令" >&2; exit 2; }

# `setsid` 让被测进程成为**新会话/新进程组的组长**（PGID == PID），
# 这样清理时 `kill -- -PGID` 只覆盖我们自己拉起来的那一棵树。
#
# **注意 `setsid` 的二次 fork**：`setsid` 只在"已经不在组长位"时直接 exec，
# 否则它会 fork 一次再让子进程 setsid。所以 `$!` 拿到的是 `setsid` 自己，
# 真正的被测进程是它的子进程 ⇒ 必须**等 cmdline 变成目标命令**再取身份，
# 否则记下的是 `setsid`/`sleep` 这类过渡态（实测踩过：哈希对不上，
# safe-kill 因此拒绝清理，这是**正确**的保守行为，但会让 pidfile 变脏）。
setsid nohup "$@" >"$LOG" 2>&1 </dev/null &
launcher=$!

# 等真正的被测进程出现：`$launcher` 自己活着且 cmdline 已等于目标，
# 或者 `$launcher` 已退出、它的子进程继承了进程组（PGID == launcher）。
want_hash=$(printf '%s\0' "$@" | sha256sum | cut -d' ' -f1)
pid=""
tries=0
while [ "$tries" -lt 400 ]; do
  tries=$((tries + 1))
  if [ -r "/proc/$launcher/stat" ]; then
    # 目标进程可能直接就是 launcher（setsid 未二次 fork 的情形）。
    if [ -r "/proc/$launcher/cmdline" ]; then
      got=$(tr '\0' ' ' <"/proc/$launcher/cmdline")
      want=$(printf '%s ' "$@")
      if [ "$got" = "$want" ]; then pid=$launcher; break; fi
    fi
    # 否则在它的子进程里找同一个进程组、cmdline 匹配的那个。
    pgid=$(awk '{print $5}' "/proc/$launcher/stat")
    for child in $(awk '{print $4}' "/proc/$pgid/task/$pgid/children" 2>/dev/null); do
      [ -r "/proc/$child/cmdline" ] || continue
      got=$(tr '\0' ' ' <"/proc/$child/cmdline")
      want=$(printf '%s ' "$@")
      if [ "$got" = "$want" ]; then pid=$child; break; fi
    done
    [ -n "$pid" ] && break
  else
    # launcher 已退出：pidfile 里还留着的话，用 PSI 反查同一进程组。
    break
  fi
  sleep 0.01
done
if [ -z "$pid" ]; then
  # 兜底：仍用 launcher，但把 cmdline 记成"启动时实际值"，并标注 readonly-warning。
  pid=$launcher
  echo "safe-spawn: 警告：未观测到目标 cmdline（记录 launcher $pid，可能已被 setsid 二次 fork）" >&2
fi
if [ ! -r "/proc/$pid/stat" ]; then
  echo "safe-spawn: 进程 $pid 启动后不可见" >&2
  exit 1
fi

starttime=$(awk '{print $22}' "/proc/$pid/stat")
cmdline_hash=$(tr '\0' ' ' <"/proc/$pid/cmdline" | sha256sum | cut -d' ' -f1)
pgid=$(awk '{print $5}' "/proc/$pid/stat")
_=$want_hash

# 一条记录一行：`<pid> <pgid> <starttime> <cmdline-sha256>`
# 追加而非覆盖：一个 pidfile 里可以有多条（例如同一脚本里先后起两个）。
printf '%s %s %s %s\n' "$pid" "$pgid" "$starttime" "$cmdline_hash" >>"$PIDFILE"
printf '%s\n' "$pid"
