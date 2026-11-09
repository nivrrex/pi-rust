// F49/Y1：生成**合成**大会话（结构对标真实大会话，内容全部是合成字符）。
//
// 用法：node gen-big-session.mjs --out <文件> [--turns N] [--bash-lines N]
// 结构（真实大会话实测）：message 条目占绝对多数，其中 assistant 与 toolResult 约 1:1，
// 每个 assistant 带 1 个 toolCall（绝大多数是 bash），结果正文可达数百 KiB。
import { writeFileSync } from "node:fs";

const argv = process.argv.slice(2);
const get = (name, fallback) => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : fallback;
};
const out = get("--out", "/tmp/f49-big.jsonl");
const turns = Number.parseInt(get("--turns", "300"), 10);
const bashLines = Number.parseInt(get("--bash-lines", "120"), 10);
const cwd = get("--cwd", "/tmp/f49-work");

/** 合成 bash 输出：多行、含少量 CJK 与制表符，行数与真实量级一致。 */
function bashOutput(index, lines) {
  const rows = [];
  for (let i = 0; i < lines; i += 1) {
    rows.push(`  row ${String(i).padStart(4, "0")}  数据列 ${(index * 7 + i) % 997}  synthetic-token-${i % 13}`);
  }
  return rows.join("\n");
}

const entries = [];
const push = (type, id, parentId, extra) =>
  entries.push({ type, id, parentId, timestamp: "2026-10-03T05:51:14.162Z", ...extra });

push("session", "s-big", null, { version: 3, cwd });
let parent = null;
let index = 0;
// 真实大会话的结构特征（F49/Y1 实测占比）：`context_edit`、`custom_message`、`compaction`
// 三种条目在真实文件里分别有 804、1362、23 条。它们**不含 message**，但恢复路径要逐条处理。
const contextEdits = Number.parseInt(get("--context-edits", "0"), 10);
const customMessages = Number.parseInt(get("--custom-messages", "0"), 10);
const compactions = Number.parseInt(get("--compactions", "0"), 10);
// 真实大会话里**单条最大 689582 字节**（≈673 KiB）：一条超长工具结果。
// 恢复路径若对它做 O(n²) 的字符串拼接/复制，这条会单独决定总耗时。
const hugeKb = Number.parseInt(get("--huge-kb", "0"), 10);
for (let i = 0; i < contextEdits; i += 1) {
  push("context_edit", `ce${i}`, parent, { ops: [{ op: "synthetic", index: i }] });
}
for (let i = 0; i < customMessages; i += 1) {
  push("custom_message", `cm${i}`, parent, { customType: "synthetic", data: { index: i } });
}
for (let i = 0; i < compactions; i += 1) {
  push("compaction", `cp${i}`, parent, { summary: "合成压缩摘要", tokensBefore: 1000, tokensAfter: 100 });
}
for (let turn = 0; turn < turns; turn += 1) {
  const userId = `u${turn}`;
  push("message", userId, parent, {
    message: { role: "user", content: [{ type: "text", text: `第 ${turn} 轮请求：请执行一次合成命令。` }], timestamp: 1760000000000 },
  });
  const assistantId = `a${turn}`;
  const callId = `call_${turn}`;
  push("message", assistantId, userId, {
    message: {
      role: "assistant",
      api: "openai-completions",
      provider: "p",
      model: "m",
      content: [
        { type: "thinking", thinking: `合成思考 ${turn}：先跑命令再看结果。` },
        { type: "text", text: `执行第 ${turn} 步。` },
        { type: "toolCall", id: callId, name: "bash", arguments: { command: `run-step ${turn}` } },
      ],
      usage: { input: 1000 + turn, output: 50, cacheRead: 0, cacheWrite: 0, totalTokens: 1050 + turn, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } },
      stopReason: "toolUse",
      timestamp: 1760000001000,
    },
  });
  push("message", `r${turn}`, assistantId, {
    message: {
      role: "toolResult",
      toolCallId: callId,
      toolName: "bash",
      content: [{ type: "text", text: bashOutput(turn, bashLines) }],
      isError: turn % 37 === 0,
      timestamp: 1760000002000,
    },
  });
  parent = `r${turn}`;
  index = turn;
}
if (hugeKb > 0) {
  const rows = [];
  for (let i = 0; i < hugeKb; i += 1) {
    rows.push(`halo-${String(i).padStart(6, "0")} 一${i % 90} 二${i % 70} 三${i % 50}`);
  }
  push("message", "huge", parent, {
    message: {
      role: "toolResult",
      toolCallId: `call_${Math.max(0, turns - 1)}`,
      toolName: "bash",
      content: [{ type: "text", text: rows.join("\n") }],
      isError: false,
      timestamp: 1760000002500,
    },
  });
  parent = "huge";
}
push("message", "final", parent, {
  message: {
    role: "assistant",
    api: "openai-completions",
    provider: "p",
    model: "m",
    content: [{ type: "text", text: `全部 ${index + 1} 轮完成。` }],
    usage: { input: 10, output: 5, cacheRead: 0, cacheWrite: 0, totalTokens: 15, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } },
    stopReason: "stop",
    timestamp: 1760000003000,
  },
});

writeFileSync(out, entries.map((entry) => JSON.stringify(entry)).join("\n") + "\n");
console.log(`${out}（${entries.length} 条，${index + 1} 轮）`);
