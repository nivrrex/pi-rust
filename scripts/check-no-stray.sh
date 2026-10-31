#!/usr/bin/env bash
# P7-6/W0(a)：残留进程检查——发现 mock-provider / 差分运行器 / pi-rs 残留即非零退出。
#
# 为什么需要：P7-5d/P7-5e 两次"假失败"都由**残留 mock-provider** 造成（抢端口、串流），
# 表现为随机场景 different，极难与真实回退区分。本脚本接入 precommit 与全量回归开头。
#
# ⚠ **不要盲目 `kill -9`**：`target/debug/pi-rs` 也可能是**人工正在用的会话**（P7-9 真机
# 反馈里就这样误杀过一次）。所以：
#   - 默认只**报告**（进程号、存活时长、工作目录、命令行），并以 1 退出让流水线停下；
#   - `--kill-difftest` 只杀"命令行里带 scenarios-tui/ 或 --scenario"的**自动化**进程。
# 注意：pgrep 的模式用 `mock[-]provider` 形式，避免匹配到本脚本自身的命令行。
set -uo pipefail
PATTERN="mock[-]provider|difftest/run\.mjs|target/debug/pi-rs"
KILL_DIFFTEST=0
[ "${1:-}" = "--kill-difftest" ] && KILL_DIFFTEST=1

# 把自己和祖先进程排除掉：脚本常被"命令行里带 mock-provider / pi-rs 字样"的
# `bash -c '…'` 调用（例如 `cargo build -p mock-provider`），否则会误报自己。
ancestors="$(pid=$$; while [ -n "${pid}" ] && [ "${pid}" != "0" ]; do printf '%s\n' "${pid}"; pid="$(ps -o ppid= -p "${pid}" 2>/dev/null | tr -d ' ')"; done)"
stray="$(pgrep -a -f "${PATTERN}" 2>/dev/null | grep -v "check-no-stray" || true)"
filtered=""
while IFS= read -r line; do
  [ -n "${line}" ] || continue
  pid="${line%% *}"
  if printf '%s\n' "${ancestors}" | grep -qx "${pid}"; then
    continue
  fi
  filtered="${filtered}${line}"$'\n'
done <<< "${stray}"
stray="${filtered}"
if [ -z "${stray}" ]; then
  printf '  无残留进程\n'
  exit 0
fi

printf '残留进程（确认不是你在用的会话！）：\n'
while read -r pid rest; do
  [ -n "${pid}" ] || continue
  age="$(ps -o etimes= -p "${pid}" 2>/dev/null | tr -d ' ')"
  cwd="$(readlink "/proc/${pid}/cwd" 2>/dev/null || echo '?')"
  printf '  pid=%s 存活=%ss cwd=%s\n    %s\n' "${pid}" "${age:-?}" "${cwd}" "$(printf '%s' "${rest}" | cut -c1-100)"
done <<< "${stray}"

if [ "${KILL_DIFFTEST}" = "1" ]; then
  printf '只清理**自动化**进程（命令行含 scenarios-tui/ 或 --scenario）：\n'
  while read -r pid rest; do
    case "${rest}" in
      *scenarios-tui/*|*--scenario*) kill -9 "${pid}" 2>/dev/null && printf '  killed %s\n' "${pid}" ;;
      *) printf '  跳过 %s（看起来不是自动化进程，请人工确认）\n' "${pid}" ;;
    esac
  done <<< "${stray}"
  exit 0
fi

# **只把自动化残留当失败**：命令行里带 `scenarios-tui/`、`--scenario`、`difftest/run.mjs`
# 或 `mock-provider` 的才是差分/回归留下的；裸 `target/debug/pi-rs`（尤其 cwd 在
# <SMOKE_DIR>）通常是人工冒烟会话，**不算失败**，只提示。
blocking=0
while read -r pid rest; do
  [ -n "${pid}" ] || continue
  case "${rest}" in
    *scenarios-tui/*|*--scenario*|*difftest/run.mjs*|*mock-provider*) blocking=1 ;;
    *) : ;;
  esac
done <<< "${stray}"

if [ "${blocking}" = "0" ]; then
  printf '以上看起来都是**人工会话**（非差分自动化），不算残留失败；如需清理请自行 Ctrl+D。\n'
  exit 0
fi

printf '清理（先确认）：自动化进程可用 %s --kill-difftest；人工会话请自己 Ctrl+D 或 kill。\n' "$0"
exit 1
