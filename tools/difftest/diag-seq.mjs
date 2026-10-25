#!/usr/bin/env node
// tools/difftest/diag-seq.mjs —— **只读**诊断：比较两侧的**事件类型序列**。
//
// 用途：当某场景的**载荷**存在"比验收判据更严的 runner 诊断"时（例如 golden/S13g 的
// `stableLayers.B=false` 只因 `message_update` 的顶层 `usage` 抖动，而该字段在验收 B 层
// 被 Q18 删除），用**类型序列**这一与判据无关的维度给出补充证据——尤其是
// `compaction_start`/`compaction_end` 与 `agent_start` 的**相对位置**。
//
// **不改判据**：本脚本不 import 比对器、不写文件、只打印；它的结论只能作为补充证据。
//
// 用法：node tools/difftest/diag-seq.mjs <场景> --ts <目录1>[,<目录2>...] --rs <目录>
//   其中每个目录都是"场景目录"（直接含 events.jsonl）。

import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const scenario = args[0];
const tsIndex = args.indexOf("--ts");
const rsIndex = args.indexOf("--rs");
if (!scenario || tsIndex < 0 || rsIndex < 0) {
  console.error("用法：node tools/difftest/diag-seq.mjs <场景> --ts <目录...> --rs <目录>");
  process.exit(2);
}
const tsDirs = args[tsIndex + 1].split(",");
const rsDir = args[rsIndex + 1];

/** 一行事件的类型；`response` 带上命令名（rpc 场景的关键信息）。 */
const kindOf = (value) => {
  if (!value || typeof value !== "object") return "?";
  if (value.type === "response") return `response:${value.command ?? "?"}`;
  if (value.type === "compaction_start" || value.type === "compaction_end") {
    return `${value.type}:${value.reason ?? "?"}`;
  }
  return value.type ?? "?";
};

const sequenceOf = (dir) => {
  const file = path.join(dir, "events.jsonl");
  return fs
    .readFileSync(file, "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => {
      try {
        return kindOf(JSON.parse(line));
      } catch {
        return "<解析失败>";
      }
    });
};

const rel = (seq) => ({
  compactionStart: seq.findIndex((k) => k.startsWith("compaction_start")),
  compactionEnd: seq.findIndex((k) => k.startsWith("compaction_end")),
  agentStart: seq.findIndex((k) => k === "agent_start"),
});

const describe = (label, seq) => {
  const r = rel(seq);
  const before = r.compactionStart >= 0 && r.agentStart >= 0 && r.compactionStart < r.agentStart;
  console.log(`\n[${label}] 共 ${seq.length} 条：${seq.join(" → ")}`);
  console.log(
    `  compaction_start@${r.compactionStart} compaction_end@${r.compactionEnd} agent_start@${r.agentStart}` +
      ` ⇒ 压缩在 agent_start 之前：${before}`,
  );
  return { seq, before };
};

const ts = tsDirs.map((dir) => describe(`TS ${dir}`, sequenceOf(dir)));
const rs = describe(`pi-rs ${rsDir}`, sequenceOf(rsDir));

const tsAllSame = ts.every((entry) => entry.seq.join("|") === ts[0].seq.join("|"));
console.log(`\nTS 各次序列彼此一致：${tsAllSame}`);
console.log(`TS 与 pi-rs 序列一致：${ts[0].seq.join("|") === rs.seq.join("|")}`);
console.log(`两侧都在 agent_start 之前压缩：${ts.every((e) => e.before) && rs.before}`);
