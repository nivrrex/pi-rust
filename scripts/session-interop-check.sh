#!/bin/sh
# 会话双向互通检查（pi-rs ⇄ TS 0.87.1）：树与可见上下文必须**逐行相等**。
#
# 为什么要有这个脚本：`tools/difftest/session-interop.mjs` 需要手工拼参数
# （`dump|produce <file> [cwd]` + `TS_PI_PACKAGE_DIR`），F62 时是手敲的，
# 后续改动无法自动复核。这里把三种形状固化成一条命令：
#   ① TS 读 pi-rs 的静态夹具；② TS 生成会话 → pi-rs 读；③ 两侧 `tree`/`context` 比对。
#
# 用法：`TS_PI_PACKAGE_DIR=<pnpm pi-coding-agent 包目录> sh scripts/session-interop-check.sh`
set -eu

[ -n "${TS_PI_PACKAGE_DIR:-}" ] || {
  echo "session-interop-check: 需要 TS_PI_PACKAGE_DIR（pnpm 的 pi-coding-agent 包目录）" >&2
  exit 2
}
REPO=$(cd "$(dirname "$0")/.." && pwd)
BIN="$REPO/target/debug/session-dump"
[ -x "$BIN" ] || {
  echo "session-interop-check: 先构建 $BIN（cargo build -p pi-cli --bin session-dump）" >&2
  exit 2
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# 静态夹具：覆盖 message / model_change / thinking_level_change / session_info 四种条目。
cat > "$WORK/fixture.jsonl" <<'JSONL'
{"type":"session","version":3,"id":"01a1f000-0000-7000-8000-000000000001","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"model_change","id":"m1","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","provider":"p","modelId":"m1"}
{"type":"thinking_level_change","id":"t1","parentId":"m1","timestamp":"2026-01-01T00:00:02.000Z","thinkingLevel":"high"}
{"type":"message","id":"u1","parentId":"t1","timestamp":"2026-01-01T00:00:03.000Z","message":{"role":"user","content":[{"type":"text","text":"hello"}],"timestamp":1000}}
{"type":"message","id":"a1","parentId":"u1","timestamp":"2026-01-01T00:00:04.000Z","message":{"role":"assistant","content":[{"type":"text","text":"hi there"}],"api":"openai-completions","provider":"p","model":"m1","timestamp":2000,"stopReason":"stop","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}}}}
{"type":"session_info","id":"s1","parentId":"a1","timestamp":"2026-01-01T00:00:05.000Z","name":"my session"}
JSONL

fail=0

compare() {
  label=$1 ts=$2 rs=$3
  if node -e '
    const fs = require("fs");
    const [tsPath, rsPath, label] = process.argv.slice(1);
    const a = JSON.parse(fs.readFileSync(tsPath, "utf8"));
    const b = JSON.parse(fs.readFileSync(rsPath, "utf8"));
    const norm = (v) => (Array.isArray(v) ? v.map((x) => (typeof x === "string" ? x : JSON.stringify(x))) : []);
    const problems = [];
    if (JSON.stringify(a.tree) !== JSON.stringify(b.tree)) problems.push("tree");
    if (JSON.stringify(norm(a.context)) !== JSON.stringify(norm(b.context))) problems.push("context");
    if (problems.length) { console.error(`${label}: ${problems.join(", ")} 不一致`); process.exit(1); }
    console.log(`${label}: tree=${a.tree.length} context=${norm(a.context).length} 一致`);
  ' "$ts" "$rs" "$label"; then :; else fail=1; fi
}

echo "=== ① TS 读 pi-rs 夹具"
node "$REPO/tools/difftest/session-interop.mjs" dump "$WORK/fixture.jsonl" "$WORK" > "$WORK/ts1.json"
"$BIN" "$WORK/fixture.jsonl" > "$WORK/rs1.json"
compare "fixture" "$WORK/ts1.json" "$WORK/rs1.json"

echo "=== ② 反向：TS 生成会话 → pi-rs 读"
node "$REPO/tools/difftest/session-interop.mjs" produce "$WORK/produced.jsonl" "$WORK" > /dev/null
node "$REPO/tools/difftest/session-interop.mjs" dump "$WORK/produced.jsonl" "$WORK" > "$WORK/ts2.json"
"$BIN" "$WORK/produced.jsonl" > "$WORK/rs2.json"
compare "produced" "$WORK/ts2.json" "$WORK/rs2.json"

echo "=== ③ pi-rs 写会话 → TS 读"
# 用 pi-rs 跑一轮真实会话（mock provider），再由 TS dump 同一份文件。
MOCK="$REPO/target/debug/mock-provider"
[ -x "$MOCK" ] || { echo "缺 $MOCK（cargo build -p mock-provider）" >&2; exit 2; }
mkdir -p "$WORK/agent" "$WORK/sess" "$WORK/work" "$WORK/home"
"$MOCK" "$REPO/scenarios/S01.json" --port 0 > "$WORK/mock.port" 2>&1 &
MOCK_PID=$!
trap 'kill "$MOCK_PID" 2>/dev/null; rm -rf "$WORK"' EXIT
port=""
i=0
while [ $i -lt 100 ]; do
  port=$(sed -n 's/^listening .*:\([0-9]*\)$/\1/p' "$WORK/mock.port" 2>/dev/null | head -1)
  [ -n "$port" ] && break
  i=$((i + 1)); sleep 0.1
done
[ -n "$port" ] || { echo "mock 未就绪" >&2; exit 2; }
printf '{"providers":{"p":{"baseUrl":"http://127.0.0.1:%s/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}' "$port" > "$WORK/agent/models.json"
printf '{"p":{"type":"api_key","key":"k"}}' > "$WORK/agent/auth.json"
printf '{"defaultProvider":"p","defaultModel":"m1"}' > "$WORK/agent/settings.json"
( cd "$WORK/work" && env HOME="$WORK/home" PI_CODING_AGENT_DIR="$WORK/agent" \
    PI_CODING_AGENT_SESSION_DIR="$WORK/sess" PI_NODE="${PI_NODE:-$(command -v node)}" \
    "$REPO/target/debug/pi-rs" --print "hello" > /dev/null 2>&1 )
find "$WORK/sess" -name '*.jsonl' -exec sh -c 'ls -S "$@" | head -1' sh {} + > "$WORK/newest" 2>/dev/null || true
written=$(head -1 "$WORK/newest" 2>/dev/null || true)
if [ -z "$written" ]; then
  echo "pi-rs 未写会话，跳过 ③"; fail=1
else
  node "$REPO/tools/difftest/session-interop.mjs" dump "$written" "$WORK" > "$WORK/ts3.json"
  "$BIN" "$written" > "$WORK/rs3.json"
  compare "pi-rs-written" "$WORK/ts3.json" "$WORK/rs3.json"
fi

if [ "$fail" -ne 0 ]; then
  echo "session-interop-check: FAILED"
  exit 1
fi
echo "session-interop-check: OK"
