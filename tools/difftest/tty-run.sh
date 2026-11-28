#!/bin/sh
# F41/P0 辅助：在**真实 pty**（script(1) 分配的伪终端）里运行一条命令。
# 目的是让被测进程的 stdout/stderr 成为 tty（复现"宿主继承 tty"这一前提），
# 输出统一落到 `<log>`，不回显到调用者终端。
# 用法：tty-run.sh <logfile> <cmd...>
log="$1"
shift
exec script -q -c "$*" "$log" >/dev/null 2>&1
