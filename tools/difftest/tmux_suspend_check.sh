#!/bin/bash
# A-d：tmux 真实 job control 下测 `ctrl+z`（app.suspend）。
# 判据（ref `interactive-mode.ts:4274-4300` handleCtrlZ）：
#   1) 挂起后 shell 的 `[1]+ 已停止 …` 必须**独占一行**（挂起前要先把光标移到内容
#      末尾下方并换行，再恢复终端模式，最后才发 SIGTSTP）；
#   2) `fg` 回来后整屏重绘，UI 不被 shell 提示符覆盖。
# 用法：tmux_suspend_check.sh "<被测命令>" <输出前缀>
# 产出：<前缀>.suspended.txt / <前缀>.resumed.txt / <前缀>.raw
set -u
BIN="$1"; OUT="$2"
S="cz-$$"
rm -f "$OUT"
tmux kill-session -t "$S" 2>/dev/null
tmux new-session -d -s "$S" -x 100 -y 30 "bash --norc -i" >/dev/null 2>&1
sleep 1
tmux pipe-pane -t "$S" "cat >> $OUT.raw"
tmux send-keys -t "$S" "PS1='$ '; clear" Enter
sleep 0.5
tmux send-keys -t "$S" "$BIN" Enter
sleep 3
tmux send-keys -t "$S" C-z
sleep 2
tmux capture-pane -p -t "$S" | sed -e 's/[[:space:]]*$//' > "$OUT.suspended.txt"
tmux send-keys -t "$S" "fg" Enter
sleep 3
tmux capture-pane -p -t "$S" | sed -e 's/[[:space:]]*$//' > "$OUT.resumed.txt"
tmux send-keys -t "$S" C-d; sleep 0.3; tmux send-keys -t "$S" "exit" Enter
sleep 0.5
tmux pipe-pane -t "$S"
tmux kill-session -t "$S" 2>/dev/null
echo "$OUT"
