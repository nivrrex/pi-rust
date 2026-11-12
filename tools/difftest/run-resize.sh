#!/bin/sh
# F54/Z3：**带窗口缩放**的单格执行器（与 `run-real.sh` 并列，不改它）。
#
# 与 `run-real.sh` 的差别只有三处：
# 1. 用 `pty-resize.py`（支持脚本里的 `resize` 步骤发 SIGWINCH）；
# 2. mark 记录里带 `cols`/`rows`（缩放前后各一段），逐步还原时按**当时的宽度**渲染；
# 3. 两侧都用「按字节偏移从 raw 还原屏幕」这一条路径（`marks-to-steps.mjs`）。
#
# 用法：
#   tools/difftest/run-resize.sh --impl ts|pi-rs --script FILE --cols 110 --rows 30 \
#       --config ext|ne --session FILE --out DIR
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
IMPL=pi-rs
SCRIPT=""
COLS=110
ROWS=30
CONFIG=ext
OUT=""
SESSION_OVERRIDE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --impl) IMPL="$2"; shift 2 ;;
    --script) SCRIPT="$2"; shift 2 ;;
    --cols) COLS="$2"; shift 2 ;;
    --rows) ROWS="$2"; shift 2 ;;
    --config) CONFIG="$2"; shift 2 ;;
    --session) SESSION_OVERRIDE="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    *) echo "run-resize: 未知参数 $1" >&2; exit 2 ;;
  esac
done
[ -n "$OUT" ] || { echo "run-resize: 缺 --out" >&2; exit 2; }
[ -n "$SCRIPT" ] || { echo "run-resize: 缺 --script" >&2; exit 2; }
case "$IMPL" in ts|pi-rs) ;; *) echo "run-resize: --impl 只接受 ts|pi-rs" >&2; exit 2 ;; esac

REALCFG=${REALCFG:-$REPO/tmp/realcfg}
[ -f "$REALCFG/env.sh" ] || { echo "run-resize: 先跑 tools/difftest/mk-realcfg.sh" >&2; exit 1; }
# shellcheck disable=SC1091
. "$REALCFG/env.sh"

TS_ENTRY=${TS_PI_ENTRY:?需要 TS_PI_ENTRY=<pi-ts>/dist/bundle/cli.js}
JITI_DIR=${PI_JITI:?需要 PI_JITI=<jiti 包目录>}
NODE_BIN=$(dirname "$(command -v node)")

rm -rf "$OUT"
mkdir -p "$OUT/home" "$OUT/mock" "$OUT/sess"
# 与 `run-real.sh` 同口径：每轮用独立副本（`--session` 会把新条目追加进会话文件），
# 并起**同一个 mock**（models.json 里的 baseUrl 指向 127.0.0.1:REALCFG_MOCK_PORT）。
cp -a "$REALCFG_AGENT" "$OUT/agent"
cp "${SESSION_OVERRIDE:-$REALCFG_SESSION_FILE}" "$OUT/session.jsonl"
cat > "$OUT/mock/scenario.json" <<EOF
{
  "id": "real-mock",
  "provider": "openai-completions",
  "model": "Ling-3.2-flash",
  "mode": "print",
  "prompt": "",
  "responses": [{ "file": "$REPO/fixtures/sse/openai/text-basic.sse" }]
}
EOF
"$REPO/target/debug/mock-provider" "$OUT/mock/scenario.json" --port "$REALCFG_MOCK_PORT" \
  > "$OUT/mock/stdout.log" 2> "$OUT/mock/stderr.log" &
MOCK_PID=$!
trap 'kill -TERM "$MOCK_PID" 2>/dev/null || true' EXIT INT TERM
python3 - "$REALCFG_MOCK_PORT" <<'PYWAIT'
import socket, sys, time
port = int(sys.argv[1])
deadline = time.time() + 10
while time.time() < deadline:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
            sys.exit(0)
    except OSError:
        time.sleep(0.02)
sys.exit(1)
PYWAIT

SESSION_FILE="$OUT/session.jsonl"
ARGS="--session $SESSION_FILE"
if [ "$CONFIG" = "ne" ]; then
  ARGS="$ARGS --no-extensions"
fi
if [ "$IMPL" = "ts" ]; then
  CMD="$TS_ENTRY $ARGS"
else
  CMD="$REPO/target/debug/pi-rs $ARGS"
fi
# 缩放脚本本身含 mark；这里不再追加（两步都由脚本显式给出，便于对照缩放前后）。
env \
  HOME="$OUT/home" \
  PATH="${HOME}/.pi/agent/bin:$NODE_BIN:/usr/local/bin:/usr/bin:/bin" \
  PI_JITI="$JITI_DIR" \
  PI_CODING_AGENT_DIR="$OUT/agent" \
  PI_CODING_AGENT_SESSION_DIR="$OUT/sess" \
  PI_PACKAGE_DIR="$(dirname "$(dirname "$(dirname "$TS_ENTRY")")")" \
  TS_PI_ENTRY="$TS_ENTRY" \
  COLORTERM= \
  LANG="${LANG:-C.UTF-8}" \
  LC_ALL="${LC_ALL:-}" \
  python3 "$REPO/tools/difftest/pty-resize.py" \
    --cols "$COLS" --rows "$ROWS" \
    --out "$OUT/raw.bin" --exit-code-file "$OUT/exit-code.txt" \
    --script "$SCRIPT" --marks "$OUT/marks.jsonl" \
    -- $CMD > "$OUT/helper.log" 2>&1 || true

# 每个 mark 按**它自己的宽度**还原屏幕（缩放前后不同）。
python3 - "$OUT" <<'PY'
import json, os, sys
out = sys.argv[1]
marks = []
for line in open(f"{out}/marks.jsonl", encoding="utf-8"):
    line = line.strip()
    if line:
        marks.append(json.loads(line))
os.makedirs(f"{out}/steps", exist_ok=True)
json.dump({"marks": marks}, open(f"{out}/steps/marks.json", "w"))
PY
node "$REPO/tools/difftest/marks-to-steps.mjs" --raw "$OUT/raw.bin" --meta "$OUT/steps/marks.json" \
  --out "$OUT/steps" --per-mark-dims >/dev/null
printf 'run-resize: %s %s %sx%s → %s\n' "$IMPL" "$CONFIG" "$COLS" "$ROWS" "$OUT"
