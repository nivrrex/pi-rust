#!/bin/sh
# F53/T1：生成**脱敏的真实配置副本** `tmp/realcfg`。
#
# 目的：让"真机才看得见"的现象（页脚分支名、thinking 档位、ctrl+o/ctrl+t 折叠、
# 扩展返回超宽行）能由 agent 在本机自动复现——用**真实**的 settings.json、
# models.json、真实 `pi-ai-summary` 扩展与真实会话文件，只替换会泄露/联网的东西。
#
# 脱敏（不改语义）：
#   1. models.json 里所有 `baseUrl` → `http://127.0.0.1:<port>/v1`；
#   2. 所有 `apiKey` → `DUMMY`（含 provider 级与模型级）；
#   3. `auth.json` **不复制**真机凭据，按同一批 provider 重新生成 `DUMMY`（type 沿用）；
#   4. `settings.json` 去掉 `shellPath`（Windows 机器路径，Linux 无效且属机器相关）。
#
# 最短必要的结构改写（写进 `<out>/SANITIZE.md`，便于复核）：
#   - 会话头 `cwd` → `<out>/work`（`git init -b <branch>` 的合成仓库，只为让页脚
#     有一个确定的分支名；会话内容一字未改）；
#   - 额外生成 `sess/nothk-*.jsonl`：同一会话去掉 `thinking_level_change`、
#     并把 `model_change` 与 assistant 消息的 provider/model 指向
#     `ant-ling/Ling-3.2-flash`（**推理模型**）。
#     用途：真实会话里最后一条 `thinking_level_change` 是 `off`，ref 与 pi-rs 都会
#     显示 `off`，无法验证"settings 的 high 是否生效"；该变体把这条排除，
#     于是 `defaultThinkingLevel=high` 必须成为唯一来源（G2 的判据）。
#
# 幂等：每次先删除再重建。收口前必须删除（`tmp/` 不入库）。
#
# 用法：
#   tools/difftest/mk-realcfg.sh [--out DIR] [--mock-port N]
#       [--settings-src FILE] [--models-src FILE] [--agent-src DIR]
#       [--session FILE] [--branch NAME] [--models-model ant-ling/Ling-3.2-flash]
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
OUT="$REPO/tmp/realcfg"
MOCK_PORT=18080
AGENT_SRC="${HOME}/.pi/agent"
WIN_REAL="/root/Desktop/tty/win-real"
SETTINGS_SRC="$AGENT_SRC/settings.json"
MODELS_SRC="$WIN_REAL/models.json"
SESSION_SRC="$WIN_REAL/2026-11-06T00-43-43-067Z_01a1ae4f-ba5b-72a9-9b2a-d6d9ad30b65e.jsonl"
BRANCH=recover
NOTHK_MODEL="ant-ling/Ling-3.2-flash"

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --mock-port) MOCK_PORT="$2"; shift 2 ;;
    --agent-src) AGENT_SRC="$2"; shift 2 ;;
    --settings-src) SETTINGS_SRC="$2"; shift 2 ;;
    --models-src) MODELS_SRC="$2"; shift 2 ;;
    --session) SESSION_SRC="$2"; shift 2 ;;
    --branch) BRANCH="$2"; shift 2 ;;
    --models-model) NOTHK_MODEL="$2"; shift 2 ;;
    *) echo "mk-realcfg: 未知参数 $1" >&2; exit 2 ;;
  esac
done

for f in "$SETTINGS_SRC" "$MODELS_SRC" "$SESSION_SRC"; do
  [ -f "$f" ] || { echo "mk-realcfg: 文件不存在：$f" >&2; exit 1; }
done
[ -d "$AGENT_SRC/extensions/pi-ai-summary" ] || {
  echo "mk-realcfg: 缺 pi-ai-summary 扩展（$AGENT_SRC/extensions）" >&2; exit 1; }

rm -rf "$OUT"
mkdir -p "$OUT/agent/extensions" "$OUT/sess" "$OUT/work"

python3 - "$SETTINGS_SRC" "$OUT/agent/settings.json" <<'PYSET'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
data.pop("shellPath", None)
json.dump(data, open(sys.argv[2], "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PYSET

python3 - "$MODELS_SRC" "$OUT/agent/models.json" "$MOCK_PORT" <<'PYMODEL'
import json, sys
src, dst, port = sys.argv[1], sys.argv[2], sys.argv[3]
data = json.load(open(src, encoding="utf-8"))
base = f"http://127.0.0.1:{port}/v1"

def scrub(entry):
    if isinstance(entry, dict):
        if "baseUrl" in entry:
            entry["baseUrl"] = base
        if "apiKey" in entry:
            entry["apiKey"] = "DUMMY"
    return entry

for provider in data.get("providers", {}).values():
    scrub(provider)
    for model in provider.get("models", []) or []:
        scrub(model)
json.dump(data, open(dst, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PYMODEL

# auth.json：不复制凭据，按 provider 生成 DUMMY。
python3 - "$AGENT_SRC/auth.json" "$OUT/agent/auth.json" "$OUT/agent/models.json" <<'PYAUTH'
import json, os, sys
auth_src, dst, models_path = sys.argv[1], sys.argv[2], sys.argv[3]
providers = list(json.load(open(models_path, encoding="utf-8")).get("providers", {}).keys())
real = {}
if os.path.exists(auth_src):
    try:
        real = json.load(open(auth_src, encoding="utf-8"))
    except json.JSONDecodeError:
        real = {}
out = {}
for name in providers:
    shape = real.get(name) if isinstance(real.get(name), dict) else {}
    kind = shape.get("type") if isinstance(shape.get("type"), str) else "api_key"
    out[name] = {"type": kind, "key": "DUMMY"}
json.dump(out, open(dst, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PYAUTH

# 真实扩展：只拷 pi-ai-summary（guard / handoff 会改键行为，不参与对拍）。
cp -a "$AGENT_SRC/extensions/pi-ai-summary" "$OUT/agent/extensions/pi-ai-summary"

# 合成 git 仓库（只为让页脚有确定的分支名）。
git -C "$OUT/work" init -q -b "$BRANCH"
printf '# sandbox\n' > "$OUT/work/AGENTS.md"
git -C "$OUT/work" add AGENTS.md >/dev/null 2>&1 || true

# 会话副本：只改会话头 cwd；另生成"无 thinking_level_change"的推理模型变体。
python3 - "$SESSION_SRC" "$OUT/sess" "$OUT/work" "$NOTHK_MODEL" <<'PYSESS'
import json, os, sys
src, sess_dir, work, model = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
provider, _, model_id = model.partition("/")
name = os.path.basename(src)
verbatim = os.path.join(sess_dir, name)
nothk = os.path.join(sess_dir, "nothk-" + name)

with open(src, encoding="utf-8", errors="replace") as fin:
    lines = fin.readlines()

# 1) 原样副本（仅 cwd）
with open(verbatim, "w", encoding="utf-8") as fout:
    for index, line in enumerate(lines):
        if index == 0 and line.strip():
            header = json.loads(line)
            header["cwd"] = work
            fout.write(json.dumps(header, ensure_ascii=False) + "\n")
            continue
        fout.write(line)

# 2) nothk 变体：丢 thinking_level_change、model_change 指向推理模型。
out = []
for index, line in enumerate(lines):
    if not line.strip():
        continue
    entry = json.loads(line)
    if index == 0 and entry.get("type") == "session":
        entry["cwd"] = work
    if entry.get("type") == "thinking_level_change":
        continue
    if entry.get("type") == "model_change":
        entry["provider"] = provider
        entry["modelId"] = model_id
    # assistant 消息里也带 provider/model，ref 的 `getSessionContextSettings` 取"更晚者"
    # ⇒ 不改它的话有效模型仍是会话末条 assistant 的那个（非推理模型），G2 无从验证。
    message = entry.get("message")
    if isinstance(message, dict) and message.get("role") == "assistant":
        message["provider"] = provider
        message["model"] = model_id
    out.append(json.dumps(entry, ensure_ascii=False))
with open(nothk, "w", encoding="utf-8") as fout:
    fout.write("\n".join(out) + "\n")

print(name)
PYSESS

SESSION_NAME=$(basename "$SESSION_SRC")

cat > "$OUT/env.sh" <<EOF
# shellcheck shell=sh
REALCFG=$OUT
REALCFG_AGENT=$OUT/agent
REALCFG_SESS=$OUT/sess
REALCFG_WORK=$OUT/work
REALCFG_SESSION_FILE=$OUT/sess/$SESSION_NAME
REALCFG_SESSION_NOTHk=$OUT/sess/nothk-$SESSION_NAME
REALCFG_MOCK_PORT=$MOCK_PORT
REALCFG_BRANCH=$BRANCH
EOF

cat > "$OUT/env.json" <<EOF
{
  "out": "$OUT",
  "agent": "$OUT/agent",
  "sess": "$OUT/sess",
  "work": "$OUT/work",
  "sessionFile": "$OUT/sess/$SESSION_NAME",
  "sessionNotHk": "$OUT/sess/nothk-$SESSION_NAME",
  "mockPort": $MOCK_PORT,
  "branch": "$BRANCH"
}
EOF

cat > "$OUT/SANITIZE.md" <<EOF
# realcfg 脱敏说明

- settings.json 来源：\`$SETTINGS_SRC\`
- models.json 来源：\`$MODELS_SRC\`（与该会话同源的那份）
- 扩展来源：\`$AGENT_SRC/extensions/pi-ai-summary\`
- 会话来源：\`$SESSION_SRC\`（只读复制）

## 脱敏（不改语义）
1. models.json 所有 \`baseUrl\` → \`http://127.0.0.1:$MOCK_PORT/v1\`
2. models.json 所有 \`apiKey\` → \`DUMMY\`
3. auth.json 不复制凭据，按 provider 生成 \`{"type": <原 type>, "key": "DUMMY"}\`
4. settings.json 去掉 \`shellPath\`

## 结构改写（最小、可复核）
5. 会话头 \`cwd\` → \`$OUT/work\`（合成 git 仓库，分支 \`$BRANCH\`）
6. 另存 \`sess/nothk-*.jsonl\`：去掉 \`thinking_level_change\`、\`model_change\` → \`$NOTHK_MODEL\`
   （真实会话最后一条 thinking 档是 \`off\`，无法验证 settings 的 \`defaultThinkingLevel=high\`）

未改写：settings 其余项、模型能力（reasoning/contextWindow/cost）、扩展源码、会话内容。

收口（提交/发布）前必须删除本目录；\`tmp/\` 不入库。
EOF

printf 'realcfg: %s (branch=%s, port=%s)\n' "$OUT" "$BRANCH" "$MOCK_PORT"
