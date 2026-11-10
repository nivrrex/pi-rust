#!/bin/sh
# F53/T3/T4 的**单格执行器**：在同一个 realcfg、同一个会话、同一宽度、同一按键序列下，
# 跑 TS 或 pi-rs，产出**逐步屏幕文本** `step-N.txt`（TS 由 `marks-to-steps.mjs` 还原，
# pi-rs 由 `PI_TEST_DUMP` 直接写），供 `real-parity.sh` 逐步比较。
#
# 用法：
#   tools/difftest/run-real.sh --impl ts|pi-rs --keys "ctrl+o,ctrl+o" \
#       --cols 110 --rows 40 --config ext|ne --out DIR [--keep]
#
# 说明：
# - `--keys` 为空字符串 = "无按键"（只比初态）；末尾**自动追加 ctrl+d** 让进程自行退出，
#   最后一步（退出后）不参与比较（比较时只取前 `按键数+1` 步）。
# - `--config ne` 传 `--no-extensions`（两侧同参数）；`ext` 用 realcfg 里的真实扩展。
# - 会话固定用 realcfg 的 `--session <file>`（两侧同一份文件，路径绝对）。
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
IMPL=pi-rs
KEYS=""
COLS=110
ROWS=40
CONFIG=ext
OUT=""
SESSION_OVERRIDE=""
KEEP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --impl) IMPL="$2"; shift 2 ;;
    --keys) KEYS="$2"; shift 2 ;;
    --cols) COLS="$2"; shift 2 ;;
    --rows) ROWS="$2"; shift 2 ;;
    --config) CONFIG="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --session) SESSION_OVERRIDE="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    *) echo "run-real: 未知参数 $1" >&2; exit 2 ;;
  esac
done
[ -n "$OUT" ] || { echo "run-real: 缺 --out" >&2; exit 2; }
case "$IMPL" in ts|pi-rs) ;; *) echo "run-real: --impl 只接受 ts|pi-rs" >&2; exit 2 ;; esac
case "$CONFIG" in ext|ne) ;; *) echo "run-real: --config 只接受 ext|ne" >&2; exit 2 ;; esac

REALCFG=${REALCFG:-$REPO/tmp/realcfg}
[ -f "$REALCFG/env.sh" ] || { echo "run-real: 先跑 tools/difftest/mk-realcfg.sh" >&2; exit 1; }
# shellcheck disable=SC1091
. "$REALCFG/env.sh"

TS_ENTRY=${TS_PI_ENTRY:-/root/Desktop/pi-ts-0.87.1/dist/bundle/cli.js}
# node 目录：真实扩展是 `.ts`，宿主用 jiti 编译（缺 PI_JITI 时扩展加载会卡住/失败）。
NODE_BIN=$(dirname "$(command -v node)")
JITI_DIR=${PI_JITI:-/root/node_modules/.pnpm/jiti@2.7.0/node_modules/jiti}
[ -f "$TS_ENTRY" ] || { echo "run-real: 找不到 TS 入口 $TS_ENTRY" >&2; exit 1; }

rm -rf "$OUT"
mkdir -p "$OUT/home" "$OUT/mock" "$OUT/sess"
# 每轮用**独立副本**：`ctrl+t` 会把 `hideThinkingBlock` 写回 settings.json，
# `--session` 会把新条目追加进会话文件。共用一份 realcfg 会让第 2 轮起状态被污染
# （实测：第二轮初始 hide 变 false），也让 TS/pi-rs 互相影响。
cp -a "$REALCFG_AGENT" "$OUT/agent"
cp "${SESSION_OVERRIDE:-$REALCFG_SESSION_FILE}" "$OUT/session.jsonl"
chmod +x "$REPO/tools/difftest/pty-helper.py" 2>/dev/null || true

# mock：响应固定为一条纯文本（本批不推进回合，只保证配置可解析、端口可连）。
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
# 等端口就绪（不固定 sleep：轮询 connect）。
python3 - "$REALCFG_MOCK_PORT" <<'PY'
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
PY

ARGS="--session $OUT/session.jsonl"
if [ "$CONFIG" = "ne" ]; then
  ARGS="$ARGS --no-extensions"
fi

SCRIPT="$OUT/script.json"
if [ "$IMPL" = "ts" ]; then
  # 逐步 mark：初态 + 每个按键之后各一个 mark，最后 ctrl+d 退出。
  python3 - "$SCRIPT" "$KEYS" <<'PY'
import json, sys
path, keys = sys.argv[1], [k.strip() for k in sys.argv[2].split(",") if k.strip()]
KEYMAP = {
    "ctrl+o": "\u000f", "ctrl+t": "\u0014", "ctrl+d": "\u0004",
    "ctrl+p": "\u0010", "ctrl+k": "\u000b", "ctrl+c": "\u0003",
    "shift+tab": "\u001b[Z", "escape": "\u001b",
}
steps = []
def mark(n):
    steps.append({"waitStable": {"quietMs": 2000, "timeoutMs": 60000}})
    steps.append({"mark": f"step{n}"})
steps.append({"waitFor": {"contains": "Pi can explain", "timeoutMs": 60000}})
mark(0)
for i, key in enumerate(keys, start=1):
    steps.append({"text": KEYMAP.get(key, key)})
    mark(i)
steps.append({"text": "\u0004"})
json.dump(steps, open(path, "w"), ensure_ascii=False)
PY
  CMD="$TS_ENTRY $ARGS"
else
  # pi-rs 自己驱动按键与快照（钩子），这里只等首帧出现、然后等进程自己退出。
  python3 - "$SCRIPT" <<'PY'
import json, sys
json.dump([{"waitFor": {"contains": "Pi can explain", "timeoutMs": 120000}}], open(sys.argv[1], "w"))
PY
  CMD="$REPO/target/debug/pi-rs $ARGS"
fi

# 按键数（两侧一致，用于确定比较到第几步）。
KEY_COUNT=$(python3 -c 'import sys;print(len([k for k in sys.argv[1].split(",") if k.strip()]))' "$KEYS")
echo "$KEY_COUNT" > "$OUT/key_count"

PI_ENV=""
if [ "$IMPL" = "pi-rs" ]; then
  PI_ENV="PI_TEST_KEYS=$KEYS,ctrl+d PI_TEST_DUMP=$OUT/steps PI_TEST_TAIL=$ROWS"
fi

mkdir -p "$OUT/steps"
# shellcheck disable=SC2086
env \
  HOME="$OUT/home" \
  PATH="/root/.pi/agent/bin:$NODE_BIN:/usr/local/bin:/usr/bin:/bin" \
  PI_JITI="$JITI_DIR" \
  PI_CODING_AGENT_DIR="$OUT/agent" \
  PI_CODING_AGENT_SESSION_DIR="$OUT/sess" \
  PI_PACKAGE_DIR="$(dirname "$(dirname "$(dirname "$TS_ENTRY")")")" \
  TS_PI_ENTRY="$TS_ENTRY" \
  COLORTERM= \
  LANG="${LANG:-C.UTF-8}" \
  LC_ALL="${LC_ALL:-}" \
  $PI_ENV \
  python3 "$REPO/tools/difftest/pty-helper.py" \
    --cols "$COLS" --rows "$ROWS" \
    --out "$OUT/raw.bin" --exit-code-file "$OUT/exit-code.txt" \
    --script "$SCRIPT" --marks "$OUT/marks.jsonl" \
    -- $CMD > "$OUT/helper.log" 2>&1 || true

if [ "$IMPL" = "ts" ]; then
  node "$REPO/tools/difftest/compare-tui.mjs" >/dev/null 2>&1 || true
  # meta.json：marks → 逐步屏幕文本
  python3 - "$OUT" "$COLS" "$ROWS" <<'PY'
import json, sys
out, cols, rows = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
marks = []
for line in open(f"{out}/marks.jsonl", encoding="utf-8"):
    line = line.strip()
    if not line:
        continue
    marks.append(json.loads(line))
json.dump({"cols": cols, "rows": rows, "snapshotOffsets": marks}, open(f"{out}/meta.json", "w"))
PY
  rm -rf "$OUT/steps"
  node "$REPO/tools/difftest/marks-to-steps.mjs" --raw "$OUT/raw.bin" --meta "$OUT/meta.json" --out "$OUT/steps"
fi

if [ "$KEEP" -eq 0 ] && [ -s "$OUT/exit-code.txt" ]; then :; fi
printf 'run-real: %s %s %sx%s keys=%s → %s\n' "$IMPL" "$CONFIG" "$COLS" "$ROWS" "$KEYS" "$OUT"
