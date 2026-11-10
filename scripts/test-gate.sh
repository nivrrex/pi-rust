#!/bin/sh
# F53b：workspace 测试的**唯一判据**。
#
# 背景：F53 中途多次用 `cargo test -q --workspace | awk` 汇总 "test result:" 行，
# 而 `test result: FAILED.` 与 `test result: ok.` 的字段位置不同，
# awk 会把失败组算进通过组 —— 于是 1083/0 这类结论掩盖了真实失败
# （`cached_frame_keeps_tool_header_spans` 就因此红了 6 个提交没被发现）。
#
# 本脚本的判据（两条，任一不满足即失败）：
#   1. `cargo test --workspace --no-fail-fast` 的**进程退出码**为 0；
#   2. 输出里 grep `FAILED|panicked|failed;` 结果为空。
# 失败时打印失败的测试名（取自 `---- <name> stdout ----`）与相关行。
#
# 用法：scripts/test-gate.sh        （日志落在 /tmp/test-gate.log，可用 TEST_GATE_LOG 覆盖）
set -u
REPO=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO" || exit 2
export PATH="${HOME}/.cargo/bin:${PATH}"
LOG=${TEST_GATE_LOG:-/tmp/test-gate.log}

cargo test --workspace --no-fail-fast >"$LOG" 2>&1
status=$?

# 判据 2：逐行 grep，非零即失败（覆盖 "exit code 掩盖了 panic" 的情况）。
# 注意：`test result: ok. N passed; 0 failed;` 里也含 "failed;"，必须只认非零。
hits=$(grep -Ec 'FAILED|panicked at|: [1-9][0-9]* failed;' "$LOG" || true)
hits=${hits:-0}

ok_groups=$(grep -c '^test result: ok\.' "$LOG" || true)
ok_groups=${ok_groups:-0}
passed=$(sed -n 's/^test result: ok\. \([0-9]\{1,\}\) passed.*/\1/p' "$LOG" | paste -sd+ - | bc 2>/dev/null || echo 0)

if [ "$status" -ne 0 ] || [ "$hits" -ne 0 ]; then
  echo "test-gate: 失败（cargo 退出码 $status，命中 FAILED/panicked/非零 failed 共 $hits 行）"
  grep -E '^---- .* stdout ----$' "$LOG" | sed -e 's/^---- /  失败用例: /' -e 's/ stdout ----$//' | sort -u
  grep -E 'FAILED|panicked at|^error(\[|:)|: [1-9][0-9]* failed;' "$LOG" | head -20
  echo "  完整日志：$LOG"
  exit 1
fi

echo "test-gate: 通过（cargo 退出码 0，test result: ok 组 $ok_groups 个，passed 合计 ${passed:-0}，FAILED/panicked 命中 0）"
