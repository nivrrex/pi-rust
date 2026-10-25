#!/usr/bin/env node
// tools/oracle/compaction-oracle.mjs —— P5-1 的 **TS oracle**。
//
// 目的：把 `compaction` 纯函数的期望值**从 TS 实现里录出来**，Rust 侧只做"与它逐字段一致"，
// 不允许由 Rust 自己写期望（RUNBOOK-6 P5.1d）。
//
// 调用路径（P5-0b 已探针确认；**不是**包入口 —— 包入口缺 5 个函数）：
//   dist/core/compaction/compaction.js
//   dist/core/compaction/utils.js
//
// 用法：
//   node tools/oracle/compaction-oracle.mjs > fixtures/oracle/compaction-p5-1.json
//
// **样本自检**：每个样本先断言"确实命中了目标分支"（否则抛错），避免空转。
import { createRequire } from "node:module";

const PKG = process.env.PI_TS_DIST ?? (() => { throw new Error("请设置 PI_TS_DIST=<pi-ts 的 dist 目录>"); })()/core/compaction";
const {
  DEFAULT_COMPACTION_SETTINGS,
  calculateContextTokens,
  estimateContextTokens,
  estimateProjectedContextTokens,
  estimateTokens,
  findCutPoint,
  findTurnStartIndex,
  prepareCompaction,
  shouldCompact,
} = await import(`${PKG}/compaction.js`);
const { buildSessionProjection } = await import(
  `${PKG}/index.js"
);
const { createFileOps, extractFileOpsFromMessage, computeFileLists, formatFileOperations, serializeConversation } =
  await import(`${PKG}/utils.js`);

const fail = (message) => {
  throw new Error(`样本自检失败：${message}`);
};

const usage = (input, output, cacheRead = 0, cacheWrite = 0) => ({
  input,
  output,
  cacheRead,
  cacheWrite,
  totalTokens: input + output + cacheRead + cacheWrite,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
});

const user = (text) => ({ role: "user", content: text });
const assistantText = (text, u) => ({
  role: "assistant",
  content: [{ type: "text", text }],
  usage: u,
  stopReason: "stop",
});
const assistantToolCall = (name, args) => ({
  role: "assistant",
  content: [{ type: "toolCall", id: "call_1", name, arguments: args }],
  usage: usage(10, 2),
  stopReason: "toolUse",
});
const toolResult = (text) => ({ role: "toolResult", toolCallId: "call_1", content: [{ type: "text", text }] });

const entry = (id, message, type = "message") => ({ type, id, parentId: null, timestamp: "2026-01-01T00:00:00.000Z", message });
const compactionEntry = (id) => ({ type: "compaction", id, parentId: null, timestamp: "2026-01-01T00:00:00.000Z", summary: "旧摘要", firstKeptEntryId: id, tokensBefore: 1 });

// ---------------------------------------------------------------------------
// ① token 估算与阈值
// ---------------------------------------------------------------------------

// 覆盖：字符串 content、块数组（text+image）、sections/toolsAdded、assistant 的 thinking/toolCall、
// toolResult、bashExecution、summary 两类。
const messages = {
  systemPlain: { role: "system", content: "x".repeat(400) },
  systemWithSectionsAndTools: {
    role: "system",
    content: "y".repeat(40),
    sections: { preamble: "a".repeat(40), tools: "" , cwd: null },
    toolsAdded: [{ name: "read", description: "d" }],
  },
  userString: user("u".repeat(80)),
  userWithImage: { role: "user", content: [{ type: "text", text: "z".repeat(8) }, { type: "image", mimeType: "image/png", data: "AAAA" }] },
  assistantMixed: {
    role: "assistant",
    content: [
      { type: "thinking", thinking: "t".repeat(12) },
      { type: "text", text: "s".repeat(16) },
      { type: "toolCall", name: "read", arguments: { path: "a.txt" } },
    ],
    usage: usage(1, 1),
    stopReason: "stop",
  },
  toolResultShort: toolResult("r".repeat(20)),
  bashExecution: { role: "bashExecution", command: "c".repeat(10), output: "o".repeat(30) },
  branchSummary: { role: "branchSummary", summary: "b".repeat(24) },
  compactionSummary: { role: "compactionSummary", summary: "p".repeat(24) },
};

const tokenSamples = {};
for (const [name, message] of Object.entries(messages)) {
  tokenSamples[name] = estimateTokens(message);
}
// 自检：图片样本必须走 4800 那一支（否则样本没覆盖到）。
if (tokenSamples.userWithImage !== Math.ceil((8 + 4800) / 4)) {
  fail(`图片估算没有走 ESTIMATED_IMAGE_CHARS=4800：${tokenSamples.userWithImage}`);
}
// 自检：空文本块与有文本块必须不同（负向对照）。
const emptyBlock = estimateTokens({ role: "user", content: [{ type: "text", text: "" }] });
const filledBlock = estimateTokens({ role: "user", content: [{ type: "text", text: "abc" }] });
if (emptyBlock === filledBlock) fail("空文本块与有文本块的估算相同（负向对照失效）");

const contextEstimateSamples = {
  // 末条 assistant 带 usage：tokens = usageTokens + trailing
  withUsage: estimateContextTokens([user("a".repeat(40)), assistantText("b".repeat(20), usage(100, 50, 10, 5)), user("c".repeat(12))]),
  // 没有可用 usage（aborted / error / 全零）：全部走估算
  abortedOnly: estimateContextTokens([
    { role: "assistant", content: [{ type: "text", text: "x".repeat(40) }], usage: usage(9, 9), stopReason: "aborted" },
    user("d".repeat(8)),
  ]),
  zeroUsage: estimateContextTokens([{ role: "assistant", content: [{ type: "text", text: "x".repeat(40) }], usage: usage(0, 0), stopReason: "stop" }]),
  empty: estimateContextTokens([]),
};
if (contextEstimateSamples.withUsage.lastUsageIndex !== 1) fail("withUsage 没命中那条 usage");
if (contextEstimateSamples.zeroUsage.lastUsageIndex !== null) fail("全零 usage 不应被当成有效 usage");

const window = 1000;
const reserve = 200;
const thresholdSamples = {
  // 阈值 = contextWindow - reserveTokens = 800；取"刚好等于 / 大 1 / 小 1"三点
  exact: shouldCompact(800, window, { enabled: true, reserveTokens: reserve, keepRecentTokens: 0 }),
  above: shouldCompact(801, window, { enabled: true, reserveTokens: reserve, keepRecentTokens: 0 }),
  below: shouldCompact(799, window, { enabled: true, reserveTokens: reserve, keepRecentTokens: 0 }),
  disabled: shouldCompact(9999, window, { enabled: false, reserveTokens: reserve, keepRecentTokens: 0 }),
};
if (thresholdSamples.exact !== false || thresholdSamples.above !== true || thresholdSamples.disabled !== false) {
  fail(`阈值边界样本不符预期：${JSON.stringify(thresholdSamples)}`);
}

const calculatedTokens = {
  nativeTotal: calculateContextTokens(usage(1, 2, 3, 4)), // totalTokens=10 → 10
  fallback: calculateContextTokens({ input: 1, output: 2, cacheRead: 3, cacheWrite: 4, totalTokens: 0, cost: {} }),
};

// ---------------------------------------------------------------------------
// ② cut point / turn 边界
// ---------------------------------------------------------------------------
// 会话条目：u1 / a1(toolCall) / toolResult / a2 / u2 / a3 / compaction / u3
const entries = [
  entry("e1", user("u".repeat(40))),
  entry("e2", assistantToolCall("read", { path: "a.txt" })),
  entry("e3", toolResult("r".repeat(40))),
  entry("e4", assistantText("a".repeat(40), usage(10, 10))),
  entry("e5", user("v".repeat(40))),
  entry("e6", assistantText("b".repeat(40), usage(10, 10))),
  compactionEntry("e7"),
  entry("e8", user("w".repeat(40))),
];

// 自检：条目里确实有 toolResult 与 compaction（否则"不切在 toolResult"/"跳过 compaction"没被覆盖）
if (!entries.some((e) => e.message?.role === "toolResult")) fail("样本缺 toolResult 条目");
if (!entries.some((e) => e.type === "compaction")) fail("样本缺 compaction 条目");

const cutSamples = {
  // 预算很小：应当切在很靠前的地方
  small: findCutPoint(entries, 0, entries.length, 1),
  // 预算很大：切点被推到最靠后的有效切点
  large: findCutPoint(entries, 0, entries.length, 10_000),
  // 切在 assistant（含 toolCall）处 → 其后 toolResult 会被保留
  atAssistant: findCutPoint(entries, 0, 4, 1),
  // 切在 user 处 → 不是 split turn（预算远大于该区间的估算量 ⇒ 落到 cutPoints[0] = e1(user)）
  atUser: findCutPoint(entries, 0, 2, 10_000),
  // 空区间 → 兜底 startIndex
  emptyRange: findCutPoint([], 0, 0, 100),
};
// 预算用尽处正好落在 toolResult 上：正确规则应切到它**之前**的有效切点，
// 错误规则（把 toolResult 也算 cut point）会切到 toolResult 自己 ⇒ 两者可区分。
const toolResultOnlyEntries = [
  entry("t1", user("u".repeat(40))),
  entry("t2", assistantText("a".repeat(40), usage(10, 10))),
  entry("t3", toolResult("r".repeat(4000))),
];
cutSamples.onlyToolResultCanSatisfy = findCutPoint(toolResultOnlyEntries, 0, toolResultOnlyEntries.length, 1);
if (cutSamples.onlyToolResultCanSatisfy.firstKeptEntryIndex === 2) {
  fail(`样本设计错：正确规则下切点不该是 toolResult（实际 ${JSON.stringify(cutSamples.onlyToolResultCanSatisfy)}）`);
}

const turnStartSamples = {
  fromToolResult: findTurnStartIndex(entries, 2, 0), // toolResult 属于 e1 那个 turn
  fromAssistant: findTurnStartIndex(entries, 3, 0),
  fromUser: findTurnStartIndex(entries, 4, 0),
  noneBefore: findTurnStartIndex(entries, 0, 2),
};

// split turn 自检：必须至少有一个样本 `isSplitTurn === true` 且 `turnStartIndex !== -1`
const splitTurns = Object.values(cutSamples).filter((r) => r.isSplitTurn);
if (splitTurns.length === 0) fail("没有任何样本产生 split-turn（覆盖不足）");
// 负向对照：`atUser` 落在 user 条目上 → 不得判为 split turn
if (cutSamples.atUser.isSplitTurn !== false) fail("切在 user 条目上却判成了 split-turn");
// 负向对照：切点绝不能落在 toolResult 条目（e3, index 2）上
for (const [name, result] of Object.entries(cutSamples)) {
  if (result.firstKeptEntryIndex === 2) fail(`${name} 切在了 toolResult 条目上`);
}

// ---------------------------------------------------------------------------
// ③ 文件操作与序列化
// ---------------------------------------------------------------------------
const fileOps = createFileOps();
extractFileOpsFromMessage(assistantToolCall("read", { path: "README.md" }), fileOps);
extractFileOpsFromMessage(assistantToolCall("edit", { path: "src/main.rs" }), fileOps);
extractFileOpsFromMessage(user("与文件无关的消息"), fileOps);
const fileLists = computeFileLists(fileOps);
const formattedFiles = formatFileOperations(fileLists.readFiles, fileLists.modifiedFiles);

const longText = "L".repeat(2500);
const serializeSamples = {
  mixed: serializeConversation([
    user("问题"),
    {
      role: "assistant",
      content: [
        { type: "thinking", thinking: "想一下" },
        { type: "text", text: "回答" },
        { type: "toolCall", name: "read", arguments: { path: "a.txt", offset: 1 } },
      ],
    },
    toolResult(longText),
    { role: "bashExecution", command: "ls", output: "a.txt" },
  ]),
  // 单参数工具调用：避开"Rust 的 serde_json::Map 无序（未开 preserve_order）"带来的键序差异，
  // 让 toolCall 那一支也能逐字节对拍（多参数键序差异见 notes/p5-1c.md 的已知限制）。
  singleArgToolCall: serializeConversation([
    { role: "assistant", content: [{ type: "toolCall", name: "read", arguments: { path: "a.txt" } }] },
  ]),
  onlyThinking: serializeConversation([{ role: "assistant", content: [{ type: "thinking", thinking: "只有思考" }] }]),
  noTextNoTool: serializeConversation([user("")]),
};
// 自检：截断样本必须真的触发截断（否则 2000 字符那一支没被覆盖）
if (!serializeSamples.mixed.includes(`[... ${longText.length - 2000} more characters truncated]`)) {
  fail("tool result 截断样本没有触发 2000 字符截断");
}
if (serializeSamples.noTextNoTool !== "") fail("空 user 文本不应产生条目");

// ---------------------------------------------------------------------------
// ④ 键序 / 数值 / 字符串（P5-1e 的 E2）
// ---------------------------------------------------------------------------
const toolCallMessage = (args) => ({
  role: "assistant",
  content: [{ type: "toolCall", name: "t", arguments: args }],
});
const argsSamples = {
  // 多参数且**非字母序**（插入序 z,a,m）
  multiArgNonAlpha: toolCallMessage({ z: 1, a: 2, m: 3 }),
  // 整数形键 + 非整数键（JS 规则：整数形键**升序在前**，其余按插入序）
  integerKeys: toolCallMessage(JSON.parse('{"10":"ten","2":"two","a":"a","1":"one"}')),
  // 重复键（JS：保留**首次位置**、取**末值**）
  duplicateKeys: toolCallMessage(JSON.parse('{"a":"first","a":"second","b":"b"}')),
  // 嵌套对象与数组
  nestedArgs: toolCallMessage({ outer: { inner: 1 }, list: [1, "two"], none: null, yes: true }),
  // 数值格式：1.0 / 1e21 / -0 / 2^53+1（JS 字面量会丢精度）/ 0.1 / 1e-7
  numberFormats: toolCallMessage({ a: 1.0, b: 1e21, c: -0, d: 9007199254740993, e: 0.1, f: 1e-7 }),
  // 字符串：U+2028 / DEL / emoji / 控制字符 / 引号与反斜杠
  stringFormats: toolCallMessage({ u2028: "\u2028", del: "\u007f", emoji: "😀", ctrl: "\u0001", quote: 'a"b\\c' }),
};
argsSamples.singleKeyNumber = toolCallMessage({ n: 1e21 });
argsSamples.singleKeyString = toolCallMessage({ s: "\u2028" });
argsSamples.singleKeyFloat = toolCallMessage({ n: 1.0 });
const serializedArgs = {};
for (const [name, message] of Object.entries(argsSamples)) {
  serializedArgs[name] = serializeConversation([message]);
}
// 样本自检：必须真的分得开"插入序 vs 字典序"等目标
if (!(serializedArgs.multiArgNonAlpha.indexOf("z=") < serializedArgs.multiArgNonAlpha.indexOf("a="))) {
  fail("multiArgNonAlpha 没保持插入序");
}
if (!(serializedArgs.integerKeys.indexOf("1=") < serializedArgs.integerKeys.indexOf("2=")
      && serializedArgs.integerKeys.indexOf("2=") < serializedArgs.integerKeys.indexOf("10=")
      && serializedArgs.integerKeys.indexOf("10=") < serializedArgs.integerKeys.indexOf("a="))) {
  fail(`integerKeys 不符合 JS 整数键升序规则：${serializedArgs.integerKeys}`);
}
if (!(serializedArgs.duplicateKeys.includes('a="second"') && !serializedArgs.duplicateKeys.includes("first"))) {
  fail(`duplicateKeys 应是"首次位置 + 末值"：${serializedArgs.duplicateKeys}`);
}
if (!serializedArgs.numberFormats.includes("1e+21")) {
  fail(`numberFormats 应出现 JS 的 1e+21：${serializedArgs.numberFormats}`);
}
if (!serializedArgs.stringFormats.includes("😀")) {
  fail(`stringFormats 应保留 emoji：${serializedArgs.stringFormats}`);
}
const argsJson = Object.fromEntries(
  Object.entries(argsSamples).map(([name, message]) => [name, message.content[0].arguments]),
);

// 单值样本（不受键序影响）：给 Rust 的 `js_stringify` 逐值对拍用
const jsValueSamples = {
  n_1_0: JSON.stringify(1.0),
  n_1e21: JSON.stringify(1e21),
  n_neg0: JSON.stringify(-0),
  n_2p53p1: JSON.stringify(9007199254740993), // JS 字面量已丢精度 ⇒ 9007199254740992
  n_0_1: JSON.stringify(0.1),
  n_1e_7: JSON.stringify(1e-7),
  n_int: JSON.stringify(42),
  n_neg: JSON.stringify(-3.5),
  n_true: JSON.stringify(true),
  n_null: JSON.stringify(null),
  s_u2028: JSON.stringify("\u2028"),
  s_del: JSON.stringify("\u007f"),
  s_emoji: JSON.stringify("😀"),
  s_ctrl: JSON.stringify("\u0001"),
  s_quote_backslash: JSON.stringify('a"b\\c'),
  s_newline_tab: JSON.stringify("a\nb\tc"),
  nested_single_key: JSON.stringify({ outer: { inner: 1 } }),
  array_mixed: JSON.stringify([1, "two", null, true]),
};

// ---------------------------------------------------------------------------
// ⑤ 投影组（P5-2pre）：用**公开入口**录像（四个私有符号由 prepareCompaction 间接覆盖）
// ---------------------------------------------------------------------------
const msg = (id, parent, role, extra = {}) => ({
  type: "message",
  id,
  parentId: parent,
  timestamp: "2026-01-01T00:00:00.000Z",
  message: { role, timestamp: 0, ...extra },
});
const assistantMsg = (id, parent, text) =>
  msg(id, parent, "assistant", {
    api: "openai-completions",
    provider: "mock",
    model: "mock-model",
    content: [{ type: "text", text }],
    usage: usage(10, 10),
    stopReason: "stop",
  });
const compactionMsg = (id, parent, firstKept, summary = "旧摘要") => ({
  type: "compaction",
  id,
  parentId: parent,
  timestamp: "2026-01-01T00:00:00.000Z",
  summary,
  firstKeptEntryId: firstKept,
  tokensBefore: 4242,
});
const customEntry = (id, parent, customType, content) => ({
  type: "custom_message",
  id,
  parentId: parent,
  timestamp: "2026-01-01T00:00:00.000Z",
  customType,
  content,
  display: true,
  details: { note: customType },
});
const branchEntry = (id, parent, summary, fromId) => ({
  type: "branch_summary",
  id,
  parentId: parent,
  timestamp: "2026-01-01T00:00:00.000Z",
  summary,
  fromId,
});
const editEntryOf = (id, parent, targetId, replacement) => ({
  type: "context_edit",
  id,
  parentId: parent,
  timestamp: "2026-01-01T00:00:00.000Z",
  targetId,
  replacement,
});

// 路径：u1 → a1 → 编辑(E1，把 u1 的内容换掉) → compaction(K，保留自 a1) → u2 → C1(custom) → B1(branch) → u3
const projectionEntries = [
  msg("u1", null, "user", { content: "第一轮问题" }),
  assistantMsg("a1", "u1", "第一轮回答"),
  editEntryOf("E1", "a1", "u1", { content: "被编辑过的问题" }),
  compactionMsg("K", "E1", "a1"),
  msg("u2", "K", "user", { content: "第二轮问题" }),
  customEntry("C1", "u2", "note", "扩展注入的一行"),
  branchEntry("B1", "C1", "分支摘要内容", "u1"),
  msg("u3", "B1", "user", { content: "第三轮问题" }),
  // 末尾放一条助手：预算很小时 cut 会落在**它**上面 ⇒ startsTurn=false ⇒ isSplitTurn=true（split turn 样本）
  assistantMsg("a3", "u3", "第三轮回答"),
];
const projection = buildSessionProjection(projectionEntries);
const projectedTokens = estimateProjectedContextTokens(projection, projectionEntries);
// `keepRecentTokens` 必须很小，否则预算用不完 ⇒ cutIndex=cutPoints[0] ⇒ messagesToSummarize 为空 ⇒ 返回 undefined
const prepared = prepareCompaction(projectionEntries, {
  ...DEFAULT_COMPACTION_SETTINGS,
  keepRecentTokens: 1,
});
// 样本自检：投影里必须含 compaction / custom / branchSummary / 被编辑的 user
const projectedRoles = projection.messages.map((message) => message.role);
for (const role of ["compactionSummary", "custom", "branchSummary", "user"]) {
  if (!projectedRoles.includes(role)) fail(`投影样本没覆盖到 role=${role}（实际 ${JSON.stringify(projectedRoles)}）`);
}
if (!prepared) fail("投影样本的 prepareCompaction 返回了 undefined");
if (!prepared.isSplitTurn) fail(`样本设计错：这个路径应当是 split turn（${JSON.stringify(prepared.firstKeptEntryId)}）`);
const projectionSample = {
  roles: projectedRoles,
  thinkingLevel: projection.thinkingLevel,
  model: projection.model,
  entryIds: projection.entries.map((entry) => entry.sourceEntry.id),
  entryMessageCounts: projection.entries.map((entry) => entry.messages.length),
  projectedTokens: {
    tokens: projectedTokens.tokens,
    usageTokens: projectedTokens.usageTokens,
    trailingTokens: projectedTokens.trailingTokens,
    lastUsageIndex: projectedTokens.lastUsageIndex,
  },
  prepared: {
    firstKeptEntryId: prepared.firstKeptEntryId,
    isSplitTurn: prepared.isSplitTurn,
    tokensBefore: prepared.tokensBefore,
    previousSummary: prepared.previousSummary ?? null,
    messagesToSummarizeRoles: prepared.messagesToSummarize.map((message) => message.role),
    messagesToSummarizeCount: prepared.messagesToSummarize.length,
    turnPrefixRoles: prepared.turnPrefixMessages.map((message) => message.role),
    fileOps: {
      read: [...prepared.fileOps.read],
      written: [...prepared.fileOps.written],
      edited: [...prepared.fileOps.edited],
    },
  },
};

// 投影组②：**不含 custom/branch**（供 Rust 在加三变体之前就能对拍投影四项）
const projectionPlainEntries = [
  msg("u1", null, "user", { content: "第一轮问题" }),
  assistantMsg("a1", "u1", "第一轮回答"),
  compactionMsg("K", "a1", "a1"),
  msg("u2", "K", "user", { content: "第二轮问题" }),
  assistantMsg("a2", "u2", "第二轮回答"),
];
const projectionPlain = buildSessionProjection(projectionPlainEntries);
const projectedTokensPlain = estimateProjectedContextTokens(projectionPlain, projectionPlainEntries);
const preparedPlain = prepareCompaction(projectionPlainEntries, {
  ...DEFAULT_COMPACTION_SETTINGS,
  keepRecentTokens: 1,
});
if (!preparedPlain) fail("投影组②的 prepareCompaction 返回 undefined");
if (!preparedPlain.isSplitTurn) fail("投影组②应当是 split turn");
const projectionSamplePlain = {
  roles: projectionPlain.messages.map((message) => message.role),
  entryIds: projectionPlain.entries.map((entry) => entry.sourceEntry.id),
  entryMessageCounts: projectionPlain.entries.map((entry) => entry.messages.length),
  projectedTokens: {
    tokens: projectedTokensPlain.tokens,
    usageTokens: projectedTokensPlain.usageTokens,
    trailingTokens: projectedTokensPlain.trailingTokens,
    lastUsageIndex: projectedTokensPlain.lastUsageIndex,
  },
  prepared: {
    firstKeptEntryId: preparedPlain.firstKeptEntryId,
    isSplitTurn: preparedPlain.isSplitTurn,
    tokensBefore: preparedPlain.tokensBefore,
    previousSummary: preparedPlain.previousSummary ?? null,
    messagesToSummarizeRoles: preparedPlain.messagesToSummarize.map((message) => message.role),
    turnPrefixRoles: preparedPlain.turnPrefixMessages.map((message) => message.role),
    fileOps: {
      read: [...preparedPlain.fileOps.read],
      written: [...preparedPlain.fileOps.written],
      edited: [...preparedPlain.fileOps.edited],
    },
  },
};

// 投影组③：直接用**golden 的会话文件**跑 prepareCompaction（S21 是 split-turn 的真实样本）
import { readFileSync } from "node:fs";
const goldenS21 = readFileSync("golden/S21/session.jsonl", "utf8")
  .split("\n")
  .filter((line) => line.trim() !== "")
  .map((line) => JSON.parse(line));
// 场景自己的压缩设置（scenarios/S21.json 的 settings.compaction）：
// `{enabled:true, reserveTokens:1000, keepRecentTokens:2000}`
const preparedS21 = prepareCompaction(goldenS21, {
  enabled: true,
  reserveTokens: 1000,
  keepRecentTokens: 2000,
});
// **证据**：golden 的会话文件里 id 是占位符（`<ID14>` 之类），`firstKeptEntryId` 指向的
// `<ID16>` 在文件里**不存在** ⇒ 树结构在归一化时丢了 ⇒ TS 自己也返回 undefined。
// 所以 P5-2 的 C 层验收必须靠**真实跑一遍**（P5-3 的压缩流程），不能拿 golden 会话硬跑。
if (preparedS21 !== undefined) {
  fail("预期 golden/S21 因占位符 id 无法投影，但 TS 却给出了 preparation");
}
const goldenS21Sample = {
  // 说明：`prepared` 恒为 null —— 占位符 id 让投影不可行（TS 与 pi-rs 都必须同样拒绝）
  prepared: null,
  placeholderIdExplanation: "golden 的 id/parentId/firstKeptEntryId 都是按出现顺序生成的占位符，引用关系不可解",
  entryKinds: goldenS21.map((entry) => entry.type),
};

const oracle = {
  goldenS21Sample,
  projectionSamplePlain,
  projectionSample,
  jsValueSamples,
  argsJson,
  serializedArgs,
  meta: {
    source: "ref/pi-0.87.1/packages/coding-agent/src/core/compaction/{compaction,utils}.ts",
    loadedFrom: PKG,
    note: "期望值来自 TS 实现（dist 模块），Rust 侧只做逐字段一致比较",
  },
  defaults: DEFAULT_COMPACTION_SETTINGS,
  calculatedTokens,
  tokenSamples,
  emptyVsFilled: { emptyBlock, filledBlock },
  contextEstimateSamples,
  thresholdSamples,
  cutSamples,
  turnStartSamples,
  fileLists: { read: fileLists.readFiles, modified: fileLists.modifiedFiles, formatted: formattedFiles },
  serializeSamples,
};

process.stdout.write(`${JSON.stringify(oracle, null, 2)}\n`);
