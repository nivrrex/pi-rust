#!/usr/bin/env bash
# 提交前的九项检查。任一失败 → 非零退出，**不得绕过**（不要用 `git commit --no-verify`）。
#
# 用途：由 .git/hooks/pre-commit 调用；也可手工 `./scripts/precommit.sh`。
# 约定（见 AGENTS.md / RUNBOOK §5）：八项必须全绿才允许提交。
#
# 注意：cargo 不在默认 PATH 中，这里显式补上；node 走 fnm 的最新版本。
set -uo pipefail
cd "$(dirname "$0")/.."

NODE_BIN="$(ls -d "$HOME"/.local/share/fnm/node-versions/*/installation/bin 2>/dev/null | tail -1 || true)"
export PATH="${NODE_BIN:+$NODE_BIN:}$HOME/.cargo/bin:$PATH"

# 平台能力探测：Windows/Git Bash 下没有 `unshare`，与网络隔离相关的检查无法执行。
# 要求：**显式提示并跳过**，不得静默通过（不改变退出码，否则 Windows 上无法提交）。
if ! command -v unshare >/dev/null 2>&1; then
  printf '=== 平台能力 ===\n'
  printf '  ⚠ 未找到 unshare：跳过所有需要网络隔离的检查（pty 探针等）。\n'
  printf '    这不是"通过"，只是本机无法执行；请在 Linux 上补跑。\n'
fi

failed=()
run() {
  local name="$1"
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
run "cargo test" cargo test --workspace
run "只读目录（ref/ 与 TS 安装包）" node scripts/check-readonly-trees.mjs
run "仓库约定（BOM/可执行位）" ./scripts/check-repo-conventions.sh
run "lint-banned" ./scripts/lint-banned.sh
run "check-spec-cites" python3 scripts/check-spec-cites.py
run "check-unverified-map" python3 scripts/check-unverified-map.py
run "check-scenarios" node scripts/check-scenarios.mjs
# P7-3：TUI 场景单独一个目录（`scenarios-tui/`），用同一校验器（**只加不减**既有八项）。
run "check-scenarios (tui)" node scripts/check-scenarios.mjs --dir scenarios-tui
# 注意：`node --test tools/difftest/test`（目录形式）会失败，必须用 glob 形式。
run "difftest node tests" node --test tools/difftest/test/*.test.mjs

# P7-4e 钉子（W1.3）：非交互路径的端到端固定断言（脚本自带头部说明与环境默认值）。
run "P7-6 残留进程检查" ./scripts/check-no-stray.sh
run "P7-4e 钉子（非交互路径 S01）" ./scripts/check-s01-pin.sh

if [ "${#failed[@]}" -gt 0 ]; then
  printf '\n十一项检查：%d 项失败 -> %s\n' "${#failed[@]}" "${failed[*]}"
  # H0.6：失败时把**完整输出**留档（只加留存，不改任何检查项与判据）
  if [ -n "${PRECOMMIT_LOG:-}" ] && [ -f "${PRECOMMIT_LOG}" ]; then
    mkdir -p notes
    cp "${PRECOMMIT_LOG}" "${PRECOMMIT_LOG}.kept"
    printf '完整输出已留档：%s\n' "${PRECOMMIT_LOG}"
  fi
  exit 1
fi
printf '\n十一项检查：全部通过\n'
