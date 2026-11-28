// F61/2：会话文件**双向互通**验证（pi-rs ⇄ TS 0.87.1）。
//
// 用途：把"pi-rs 写的会话能被 TS 读成同一棵树、同一段可见内容"与反向
// 都固化成可自动执行的检查。覆盖 F60 改过的三处风险面：会话条目链
// （`branch_summary` 的 parentId/fromId）、`compaction` 条目、树结构。
//
// 判据：两侧各自输出 `type:id:parentId` 序列与上下文消息文本，逐行相等。
// TS 侧用**真实包**的 `SessionManager`（长路径 pnpm 包），路径由
// `TS_PI_PACKAGE_DIR` 提供；没提供就跳过（不猜路径）。
"use strict";

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const pkgDir = process.env.TS_PI_PACKAGE_DIR;
if (!pkgDir) {
  console.error("需要 TS_PI_PACKAGE_DIR=<pnpm pi-coding-agent 包目录>");
  process.exit(2);
}
const require = createRequire(path.join(pkgDir, "package.json"));
const { SessionManager } = require(path.join(pkgDir, "dist/index.js"));

const mode = process.argv[2]; // "dump" | "produce"
const file = process.argv[3];
const cwd = process.argv[4];

/** 树结构：按文件顺序的 `type:id:parentId`。 */
function treeLines(manager) {
  return manager
    .getEntries()
    .map((e) => `${e.type}:${e.id}:${e.parentId === null ? "null" : e.parentId}`);
}

/** 上下文里的可见文本（`buildContextEntries` → 每条消息的文本）。 */
function contextLines(manager) {
  const out = [];
  for (const entry of manager.buildContextEntries()) {
    if (entry.type === "message") {
      const content = entry.message.content;
      const text = Array.isArray(content)
        ? content.filter((c) => c && c.type === "text").map((c) => c.text).join("")
        : typeof content === "string"
          ? content
          : "";
      out.push(`msg:${entry.message.role}:${text}`);
    } else if (entry.type === "compaction") {
      out.push(`compaction:${entry.summary}`);
    } else if (entry.type === "branch_summary") {
      out.push(`branch_summary:${entry.summary}`);
    }
  }
  return out;
}

// `SessionManager(cwd, sessionDir, sessionFile, persist)`：cwd 必填（ref 用它解析
// 相对路径），`sessionFile` 给了就直接读那一份。
// `persist`：produce 模式必须为 true，否则所有 `append*` 只留在内存、不落盘
// （实测：`persist=false` 时 produce 返回成功但文件不存在，dump 读到空内容）。
const manager = new SessionManager(
  cwd || process.cwd(),
  path.dirname(file),
  file,
  mode === "produce",
);
if (typeof manager.setSessionFile === "function" && !manager.getEntries().length) {
  manager.setSessionFile(file);
}

if (mode === "dump") {
  console.log(JSON.stringify({ tree: treeLines(manager), context: contextLines(manager) }, null, 1));
  process.exit(0);
}

if (mode === "produce") {
  // TS 侧产出一份同构会话：消息 → branch_summary（挂到导航目标）→ 压缩。
  const first = manager.appendMessage({
    role: "user",
    content: [{ type: "text", text: "第一问" }],
    timestamp: 1,
  });
  const answer = manager.appendMessage({
    role: "assistant",
    content: [{ type: "text", text: "第一答" }],
    api: "openai-completions",
    provider: "p",
    model: "m",
    usage: {
      input: 100,
      output: 10,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 110,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
    stopReason: "stop",
    timestamp: 1,
  });
  const leafA = manager.appendMessage({
    role: "user",
    content: [{ type: "text", text: "分支A的问题" }],
    timestamp: 1,
  });
  const oldLeaf = manager.appendMessage({
    role: "assistant",
    content: [{ type: "text", text: "分支A的回答" }],
    api: "openai-completions",
    provider: "p",
    model: "m",
    usage: {
      input: 100,
      output: 10,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 110,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
    stopReason: "stop",
    timestamp: 1,
  });
  // 无摘要导航到 `answer`（叶子先回到目标，再挂摘要）→ 等价于
  // `branchWithSummary(newLeafId=answer, …)`，`fromId` 是当时的旧叶子。
  manager.branchWithSummary(answer, "无摘要导航的分支摘要", {
    readFiles: [],
    modifiedFiles: [],
  });
  manager.appendMessage({
    role: "user",
    content: [{ type: "text", text: "分支B的问题" }],
    timestamp: 1,
  });
  manager.appendMessage({
    role: "assistant",
    content: [{ type: "text", text: "分支B的回答" }],
    api: "openai-completions",
    provider: "p",
    model: "m",
    usage: {
      input: 100,
      output: 10,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 110,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
    stopReason: "stop",
    timestamp: 1,
  });
  manager.appendCompaction(
    "压缩摘要正文",
    answer,
    12000,
    { readFiles: [], modifiedFiles: [] },
    undefined,
    false,
  );
  manager.appendMessage({
    role: "user",
    content: [{ type: "text", text: "压缩后的问题" }],
    timestamp: 1,
  });
  manager.appendMessage({
    role: "assistant",
    content: [{ type: "text", text: "压缩后的回答" }],
    api: "openai-completions",
    provider: "p",
    model: "m",
    usage: {
      input: 100,
      output: 10,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 110,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
    stopReason: "stop",
    timestamp: 1,
  });
  fs.mkdirSync(cwd, { recursive: true });
  console.log("produced", file, "leaf", manager.getLeafId(), "first", first, "leafA", leafA, "oldLeaf", oldLeaf);
  process.exit(0);
}

console.error("未知 mode:", mode);
process.exit(2);
