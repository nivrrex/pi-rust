#!/bin/sh
# 真机测试环境：pi-handoff-gate 的 todo（会话 appendEntry 路径）。
#
# 用法（在目标机上，仓库根目录执行）：
#   sh tools/testenv/handoff-todo.sh            # 建沙箱并打印后续命令
#   sh tools/testenv/handoff-todo.sh --with-auth  # 额外把 auth.json 复制进沙箱（默认不复制）
#   sh tools/testenv/handoff-todo.sh --check    # 只检查已有沙箱里的会话条目
#
# 特点：**不碰 ~/.pi**、不联网、不需要改任何配置；所有东西都在 $HOME/pi-testenv 下。
# 说明：脚本不硬编码用户路径；需要事先把 pi-rs 放进 PATH 或设 PI_RS=<路径>。
set -eu

BASE="${PI_TESTENV_DIR:-$HOME/pi-testenv}"
PI_RS="${PI_RS:-pi-rs}"
MODE="${1:-}"
WITH_AUTH=0
[ "$MODE" = "--with-auth" ] && WITH_AUTH=1

if [ "$MODE" = "--check" ]; then
  if [ ! -d "$BASE/sessions" ]; then
    echo "没有找到 $BASE/sessions，先执行一次：sh $0" >&2
    exit 2
  fi
  echo "== 会话条目统计（顶层 type；todo 状态在 toolResult.details.state）"
  verdict=0
  for f in "$BASE"/sessions/*.jsonl; do
    [ -f "$f" ] || continue
    echo "-- $f"
    node -e '
const fs = require("fs");
const counts = new Map();
const custom = new Map();
let todoStates = 0;
let lastTodo = null;
for (const line of fs.readFileSync(process.argv[1], "utf8").split("\n")) {
  if (!line.trim()) continue;
  let entry;
  try { entry = JSON.parse(line); } catch { continue; }
  const type = entry.type ?? "(none)";
  counts.set(type, (counts.get(type) ?? 0) + 1);
  if (entry.type === "custom" && entry.customType) {
    custom.set(entry.customType, (custom.get(entry.customType) ?? 0) + 1);
  }
  // pi-handoff-gate 的 todo：状态写在 toolResult 的 details.state（不是 appendEntry）。
  const message = entry.message;
  if (entry.type === "message" && message && message.role === "toolResult" && message.toolName === "todo") {
    if (message.details && message.details.state) {
      todoStates += 1;
      lastTodo = message.details.state;
    }
  }
}
for (const [type, n] of [...counts].sort((a, b) => b[1] - a[1])) {
  console.log(`   ${String(n).padStart(4)}  ${type}`);
}
console.log(custom.size ? `   customType: ${[...custom].map(([k, v]) => `${k}×${v}`).join(", ")}` : "   customType: （无）");
if (todoStates > 0) {
  const items = Array.isArray(lastTodo.items) ? lastTodo.items : [];
  console.log(`   todo 快照: ${todoStates} 条 toolResult.details.state；最后一条 ${items.length} 项`);
} else {
  console.log("   todo 快照: （无）");
}
process.exit(todoStates > 0 ? 0 : 1);
' "$f" && verdict=1
  done
  echo
  if [ "$verdict" = "1" ]; then
    echo "判读：**todo 状态已落盘**（toolResult.details.state，扩展在 session_start/session_tree 用 getBranch() 重建）。"
    echo "      注：pi-handoff-gate 的 todo **不依赖** appendEntry；custom 条目只在 /todo clear 时出现，属预期。"
  else
    echo "判读：**没有 todo 快照** —— 三种可能：① 本会话没有调用 todo 工具；② 扩展未加载"
    echo "      （启动屏应出现 [Extension issues] 段，或启动时 stderr 有 '未找到 jiti'/'扩展未加载'）；"
    echo "      ③ toolResult.details 没写进会话。请把上面的输出与启动时的提示一并贴回。"
  fi
  exit 0
fi

mkdir -p "$BASE/agent/extensions" "$BASE/sessions" "$BASE/work" "$BASE/home"
echo "== 沙箱：$BASE"
echo "   agent 目录: $BASE/agent"
echo "   会话目录 : $BASE/sessions"

# 复制（而不是链接）真实扩展，避免测试时写坏 ~/.pi
for ext in pi-handoff-gate pi-interact pi-secrets; do
  src="$HOME/.pi/agent/extensions/$ext"
  if [ -d "$src" ]; then
    rm -rf "$BASE/agent/extensions/$ext"
    cp -r "$src" "$BASE/agent/extensions/$ext"
    echo "   已复制扩展: $ext"
  fi
done

# 复用真实 provider 配置（只读复制），但不复制 auth.json（避免把凭据落到测试目录）
# `--with-auth`：沙箱里没有凭据时 provider 会报 "没有可用的 API key"（脚本默认不复制，避免凭据落盘）。
if [ "$WITH_AUTH" = "1" ] && [ ! -f "$HOME/.pi/agent/auth.json" ]; then
  echo "   ** 没找到 $HOME/.pi/agent/auth.json：沙箱里可能没有凭据。"
  echo "      若你的 key 来自环境变量（OPENROUTER_API_KEY 等），启动时保留该变量即可；"
  echo "      否则用：pi-rs --api-key <key> ，或手动把凭据放进 $BASE/agent/auth.json"
fi
if [ "$WITH_AUTH" = "1" ] && [ -f "$HOME/.pi/agent/auth.json" ]; then
  cp "$HOME/.pi/agent/auth.json" "$BASE/agent/auth.json"
  chmod 600 "$BASE/agent/auth.json"
  echo "   已复制 auth.json（沙箱内，权限 600）"
fi

for f in models.json settings.json; do
  if [ -f "$HOME/.pi/agent/$f" ]; then
    cp "$HOME/.pi/agent/$f" "$BASE/agent/$f"
    echo "   已复制配置: $f"
  fi
done

cat <<TIP

== 这样启动（不会碰 ~/.pi）：
  HOME=$BASE/home \\
  PI_CODING_AGENT_DIR=$BASE/agent \\
  PI_CODING_AGENT_SESSION_DIR=$BASE/sessions \\
  cd $BASE/work && $PI_RS -c

== 在 TUI 里：
  1) 让它做几件需要 todo 的事（例如「列个三步计划并跟踪」）
  2) 退出（Ctrl+D）
  3) 回来检查条目：
     sh $0 --check
  期望：会话文件里出现 custom 条目（customType 里应含 todo 相关值）
  若 --check 报"没有 custom 条目"，即 F10 那条未修缺陷复现，请把 --check 的输出贴回。

== 若启动时报 "没有可用的 API key"：重跑一次并带 --with-auth（把 ~/.pi/agent/auth.json 复制进沙箱，
   权限 600）。不复制 auth.json 时，只有 models.json 里写了 apiKey 或环境变量里有 key 才能用。
TIP
