set -eu
BASE=$(mktemp -d)
mkdir -p "$BASE/sessions"
# 脚本自身位置（不硬编码本机路径）
HERE=$(cd "$(dirname "$0")" && pwd)
# 情形 A：有 todo 快照
cat > "$BASE/sessions/a.jsonl" <<'EOF'
{"type":"session","version":3,"id":"a","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"1","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"toolResult","toolCallId":"c","toolName":"todo","content":[{"type":"text","text":"x"}],"isError":false,"timestamp":1,"details":{"action":"set","state":{"items":[{"id":1,"status":"pending","text":"a"}]}}}}
EOF
out=$(PI_TESTENV_DIR="$BASE" sh "$HERE/handoff-todo.sh" --check)
echo "$out" | grep -q "todo 状态已落盘" || { echo "FAIL A"; echo "$out"; exit 1; }
# 情形 B：没有 todo 快照
rm -f "$BASE"/sessions/*.jsonl
cat > "$BASE/sessions/b.jsonl" <<'EOF'
{"type":"session","version":3,"id":"b","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
EOF
out=$(PI_TESTENV_DIR="$BASE" sh "$HERE/handoff-todo.sh" --check)
echo "$out" | grep -q "没有 todo 快照" || { echo "FAIL B"; echo "$out"; exit 1; }
echo "PASS 两种判读分支"
rm -rf "$BASE"
