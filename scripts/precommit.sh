#!/usr/bin/env bash
# 提交前的检查清单。任一失败 → 非零退出，**不得绕过**（不要用 `git commit --no-verify`）。
# 项数不写死：`run` 会累加 `checked`，标题按实际跑了多少项输出。
#
# 用途：由 .git/hooks/pre-commit 调用；也可手工 `./scripts/precommit.sh`。
# 约定（见 AGENTS.md / RUNBOOK §5）：八项必须全绿才允许提交。
#
# 注意：cargo 不在默认 PATH 中，这里显式补上；node 走 fnm 的最新版本。
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

NODE_BIN="$(find "$HOME"/.local/share/fnm/node-versions -type d -path '*/installation/bin' -print 2>/dev/null | sort | tail -1 || true)"
export PATH="${NODE_BIN:+$NODE_BIN:}$HOME/.cargo/bin:$PATH"

# 平台能力探测：Windows/Git Bash 下没有 `unshare`，与网络隔离相关的检查无法执行。
# 要求：**显式提示并跳过**，不得静默通过（不改变退出码，否则 Windows 上无法提交）。
if ! command -v unshare >/dev/null 2>&1; then
  printf '=== 平台能力 ===\n'
  printf '  ⚠ 未找到 unshare：跳过所有需要网络隔离的检查（pty 探针等）。\n'
  printf '    这不是"通过"，只是本机无法执行；请在 Linux 上补跑。\n'
fi

failed=()
# 已跑项数：标题里的"N 项"用它，避免新增检查后标题与实际不符（F120 之前写死"十六"）。
checked=0
run() {
  local name="$1"
  checked=$((checked + 1))
  shift
  local log
  log="$(mktemp)"
  printf '=== %s ===\n' "$name"
  if "$@" >"$log" 2>&1; then
    printf '  通过\n'
  else
    printf '  **失败**（退出码 %s）——原始输出：\n' "$?"
    sed 's/^/    /' "$log" | tail -40
    failed+=("$name")
  fi
  rm -f "$log"
}

# H0.6：把完整输出同时写到临时日志；失败时由下面的分支留档到 notes/。
PRECOMMIT_LOG="$(mktemp -t precommit-XXXXXX)"
export PRECOMMIT_LOG
exec > >(tee "${PRECOMMIT_LOG}") 2>&1

run "cargo fmt --check" cargo fmt --check
run "cargo clippy" cargo clippy --workspace --all-targets -- -D warnings
# 统一走 nextest（本环境已装 cargo-nextest，约 3 倍速；doctest 不在本脚本覆盖范围）。
run "cargo nextest" cargo nextest run --workspace
run "只读目录（ref/ 与 TS 安装包）" node scripts/check-readonly-trees.mjs
run "仓库约定（BOM/可执行位）" ./scripts/check-repo-conventions.sh
run "仓库约定正负对照" ./scripts/check-repo-conventions-selftest.sh
run "lint-banned" ./scripts/lint-banned.sh
# 输出顺序不得由 HashMap/HashSet 的**随机迭代序**决定（每次进程启动都变）。
run "hash-order" ./scripts/check-hash-order.sh
run "ctx-model-shape" ./scripts/check-ctx-model-shape.sh
run "tui-stderr" ./scripts/check-tui-stderr.sh
# F56/P：禁止按名字/端口/模式批量杀进程（F55 误杀 openbox 的直接原因）。
run "kill-safety" ./scripts/check-kill-safety.sh
# F56/P：safe-spawn / safe-kill 的行为用例（PID 复用、cmdline 不符、正常清理、拒绝按名匹配）。
run "safe-tools tests" ./tools/difftest/test/test-safe-tools.sh
# F56/A4：回归汇总不得把"空清单"算成一次运行（light 的假 nogolden）。
run "regress-summary test" ./tools/difftest/test/test-regress-summary.sh
# F57/C2：shim `copyToClipboard`（`/allcopy` 依赖）的行为用例（8 项，含假命令与 OSC 52）。
run "shim clipboard tests" node --test tools/ext-host/test/shim-clipboard.test.mjs
run "check-spec-cites" python3 scripts/check-spec-cites.py
run "check-unverified-map" python3 scripts/check-unverified-map.py
run "check-scenarios" node scripts/check-scenarios.mjs
# P7-3：TUI 场景单独一个目录（`scenarios-tui/`），用同一校验器（**只加不减**既有八项）。
run "check-scenarios (tui)" node scripts/check-scenarios.mjs --dir scenarios-tui
# 注意：`node --test tools/difftest/test`（目录形式）会失败，必须用 glob 形式。
run "difftest node tests" node --test tools/difftest/test/*.test.mjs

# P7-4e 钉子（W1.3）：非交互路径的端到端固定断言（脚本自带头部说明与环境默认值）。
run "P7-6 残留进程检查" ./scripts/check-no-stray.sh
# 判定逻辑自检：F118 发现 `check-no-stray.sh` 曾把 pty-helper 拉起的 pi-rs 残留
# 判成"人工会话"放过（那条命令行里既无 `scenarios-tui/` 也无 `--scenario`），
# 而上面那条检查当时**照样通过**。这个自检用临时伪造进程钉住该判定。
run "P7-6 判定自检（pty-helper 残留必须算自动化）" ./scripts/check-no-stray-selftest.sh
run "P7-4e 钉子（非交互路径 S01）" ./scripts/check-s01-pin.sh

if [ "${#failed[@]}" -gt 0 ]; then
  printf '\n%d 项检查：%d 项失败 -> %s\n' "$checked" "${#failed[@]}" "${failed[*]}"
  # H0.6：失败时把**完整输出**留档（只加留存，不改任何检查项与判据）
  if [ -n "${PRECOMMIT_LOG:-}" ] && [ -f "${PRECOMMIT_LOG}" ]; then
    mkdir -p notes
    cp "${PRECOMMIT_LOG}" "${PRECOMMIT_LOG}.kept"
    printf '完整输出已留档：%s\n' "${PRECOMMIT_LOG}"
  fi
  exit 1
fi
printf '\n%d 项检查：全部通过\n' "$checked"
