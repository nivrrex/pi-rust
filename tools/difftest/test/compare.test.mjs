// compare.mjs 的自测：相同输入通过；改动任一层一行必须失败；缺文件必须失败。

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { existsSync as fsExists, statSync as fsStatSize } from "node:fs";
import path from "node:path";

import {
  allComparableLayersMatch,
  assertSafeOutDir,
  compareScenario,
  compareSessionLines,
  STATUS,
} from "../compare.mjs";

/** 造一对目录：golden 与 actual，内容由 `files` 决定（值相同即两侧相同）。 */
async function makePair(files) {
  const root = await mkdtemp(path.join(tmpdir(), "compare-test-"));
  const goldenDir = path.join(root, "golden", "SXX");
  const actualDir = path.join(root, "actual", "SXX");
  for (const [name, content] of Object.entries(files)) {
    for (const dir of [goldenDir, actualDir]) {
      await mkdir(dir, { recursive: true });
      await writeFile(path.join(dir, name), content);
    }
  }
  return { root, goldenDir, actualDir };
}

const BASE = {
  "events.jsonl": '{"type":"agent_start"}\n{"type":"message_update","i":1}\n{"type":"agent_end"}\n',
  "requests.jsonl": '{"model":"mock-model"}\n',
  "fs.json": "{}\n",
  "meta.json": '{"exitCode":0}\n',
};

// A 层从 P4 起纳入比较：fixture 必须带 session.jsonl，否则 A 层会判 `missing`。
BASE["session.jsonl"] =
  '{"id":"<ID1>","type":"session"}\n' +
  '{"id":"<ID2>","parentId":null,"type":"message","message":{"role":"user"}}\n';

test("identical inputs pass on every comparable layer", async () => {
  const { root, goldenDir, actualDir } = await makePair(BASE);
  const layers = await compareScenario(goldenDir, actualDir);
  assert.equal(layers.B.status, STATUS.same);
  assert.equal(layers.C.status, STATUS.same);
  assert.equal(layers.fs.status, STATUS.same);
  assert.equal(layers.exit.status, STATUS.same);
  assert.equal(layers.A.status, STATUS.same, "A 层自 P4 起纳入比较");
  assert.ok(allComparableLayersMatch(layers));
  await rm(root, { recursive: true, force: true });
});

for (const file of ["events.jsonl", "requests.jsonl", "fs.json", "session.jsonl"]) {
  test(`a single changed line in ${file} fails that layer`, async () => {
    const { root, goldenDir, actualDir } = await makePair(BASE);
    const changed = `${BASE[file]}{"extra":true}\n`;
    await writeFile(path.join(actualDir, file), changed);
    const layers = await compareScenario(goldenDir, actualDir);
    const layerName =
      file === "events.jsonl"
        ? "B"
        : file === "requests.jsonl"
          ? "C"
          : file === "session.jsonl"
            ? "A"
            : "fs";
    assert.equal(layers[layerName].status, STATUS.different, `${layerName} 必须失败`);
    assert.ok(!allComparableLayersMatch(layers));
    await rm(root, { recursive: true, force: true });
  });
}

test("a changed exit code fails", async () => {
  const { root, goldenDir, actualDir } = await makePair(BASE);
  await writeFile(path.join(actualDir, "meta.json"), '{"exitCode":1}\n');
  const layers = await compareScenario(goldenDir, actualDir);
  assert.equal(layers.exit.status, STATUS.different);
  await rm(root, { recursive: true, force: true });
});

test("a missing file on either side fails instead of counting as equal", async () => {
  const { root, goldenDir, actualDir } = await makePair(BASE);
  await rm(path.join(actualDir, "requests.jsonl"));
  const layers = await compareScenario(goldenDir, actualDir);
  assert.equal(layers.C.status, STATUS.missing, "缺文件必须报 missing");
  assert.ok(!allComparableLayersMatch(layers), "缺文件不得算通过");

  const { root: root2, goldenDir: g2, actualDir: a2 } = await makePair(BASE);
  await rm(path.join(g2, "fs.json"));
  const layers2 = await compareScenario(g2, a2);
  assert.equal(layers2.fs.status, STATUS.missing);
  await rm(root, { recursive: true, force: true });
  await rm(root2, { recursive: true, force: true });
});

test("consecutive streaming increments are folded, non-consecutive ones are not", async () => {
  const { root, goldenDir, actualDir } = await makePair(BASE);
  // 连续三条 message_update vs 一条：折叠后应相同（PLAN 第 3 节允许）。
  // 注意三行的**内容要与 golden 的一致**，只有条数不同——折叠吸收的是条数，不是内容。
  const update = '{"type":"message_update","i":1}\n';
  await writeFile(
    path.join(actualDir, "events.jsonl"),
    `{"type":"agent_start"}\n${update}${update}${update}{"type":"agent_end"}\n`,
  );
  const folded = await compareScenario(goldenDir, actualDir);
  assert.equal(folded.B.status, STATUS.same, "连续增量折叠后应一致");

  // 但**非相邻**的同类事件不能被折叠掉：多加一个 turn 必须失败。
  await writeFile(
    path.join(actualDir, "events.jsonl"),
    `{"type":"agent_start"}\n${update}{"type":"agent_end"}\n${update}`,
  );
  const extra = await compareScenario(goldenDir, actualDir);
  assert.equal(extra.B.status, STATUS.different, "非相邻增量不得被折叠掩盖");
  await rm(root, { recursive: true, force: true });
});

test("absolute or out-of-repo --out paths are rejected", () => {
  assert.throws(() => assertSafeOutDir("/tmp/x", "/repo"), /绝对路径/);
  assert.throws(() => assertSafeOutDir("../outside", "/repo"), /仓库内/);
  assert.equal(assertSafeOutDir("tmp/cmp", "/repo"), "/repo/tmp/cmp");
});

// —— Q18 的机械规则：只忽略 message_start/message_update 的 usage ——

test("streaming usage differences are ignored, terminal usage differences are not", async () => {
  // 1) `message_update` 的 usage 不同 → 仍算一致（规则放行）。
  const streaming = {
    "events.jsonl": '{"type":"message_update","usage":{"input":0},"message":{"usage":{"input":0}}}\n',
    "requests.jsonl": "{}\n",
    "fs.json": "{}\n",
    "meta.json": '{"exitCode":0}\n',
  };
  const a = await makePair(streaming);
  await writeFile(
    path.join(a.actualDir, "events.jsonl"),
    '{"type":"message_update","usage":{"input":999},"message":{"usage":{"input":999}}}\n',
  );
  const layers = await compareScenario(a.goldenDir, a.actualDir);
  assert.equal(layers.B.status, STATUS.same, "流式 usage 应被忽略");
  await rm(a.root, { recursive: true, force: true });

  // 2) `message_end` 的 usage 不同 → **必须**失败（终态数据逐位比较）。
  for (const type of ["message_end", "turn_end", "agent_end"]) {
    const terminal = {
      "events.jsonl": `{"type":"${type}","usage":{"input":10},"cost":{"total":0.5}}\n`,
      "requests.jsonl": "{}\n",
      "fs.json": "{}\n",
      "meta.json": '{"exitCode":0}\n',
    };
    const b = await makePair(terminal);
    await writeFile(
      path.join(b.actualDir, "events.jsonl"),
      `{"type":"${type}","usage":{"input":11},"cost":{"total":0.5}}\n`,
    );
    const result = await compareScenario(b.goldenDir, b.actualDir);
    assert.equal(result.B.status, STATUS.different, `${type} 的 usage 必须逐位比较`);
    await rm(b.root, { recursive: true, force: true });
  }

  // 3) `message_end` 的 cost 不同也必须失败。
  const costCase = {
    "events.jsonl": '{"type":"message_end","usage":{"input":10},"cost":{"total":0.5}}\n',
    "requests.jsonl": "{}\n",
    "fs.json": "{}\n",
    "meta.json": '{"exitCode":0}\n',
  };
  const c = await makePair(costCase);
  await writeFile(
    path.join(c.actualDir, "events.jsonl"),
    '{"type":"message_end","usage":{"input":10},"cost":{"total":0.500001}}\n',
  );
  const costResult = await compareScenario(c.goldenDir, c.actualDir);
  assert.equal(costResult.B.status, STATUS.different, "cost 必须逐位比较");
  await rm(c.root, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// A 层（session.jsonl 逐行比较）的负向对照。
//
// **对照必须"真的触发"**：先断言被改动的条目**确实存在于该样本里**，
// 否则对照会空转——我在 S01 上测 `replacement` 就踩过这个坑（S01 没有 context_edit 条目）。
// ---------------------------------------------------------------------------

const SESSION_SAMPLE = [
  '{"id":"<ID1>","type":"session","cwd":"<cwd>"}',
  '{"id":"<ID2>","parentId":null,"type":"message","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}',
  '{"id":"<ID3>","parentId":"<ID2>","type":"message","message":{"role":"assistant","content":[],"stopReason":"error"}}',
  '{"id":"<ID4>","parentId":"<ID3>","type":"context_edit","targetId":"<ID3>","replacement":null}',
  '{"id":"<ID5>","parentId":"<ID2>","type":"message","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}',
];

function withSession(lines) {
  return lines.join("\n") + "\n";
}

test("A 层：逐行相同判 same", () => {
  const result = compareSessionLines(withSession(SESSION_SAMPLE), withSession(SESSION_SAMPLE));
  assert.equal(result.status, STATUS.same);
});

test("A 层：键序不同仍判 same（Q8 键顺序不是判据）", () => {
  const reordered = [
    '{"type":"session","cwd":"<cwd>","id":"<ID1>"}',
    ...SESSION_SAMPLE.slice(1),
  ];
  const result = compareSessionLines(withSession(SESSION_SAMPLE), withSession(reordered));
  assert.equal(result.status, STATUS.same);
});

test("A 层负向对照：少一行必须 different（行数不同）", () => {
  const fewer = SESSION_SAMPLE.filter((line) => !line.includes("context_edit"));
  assert.equal(fewer.length, SESSION_SAMPLE.length - 1, "对照必须真的删掉了一条");
  const result = compareSessionLines(withSession(SESSION_SAMPLE), withSession(fewer));
  assert.equal(result.status, STATUS.different);
  assert.match(result.detail, /行数不同/);
});

test("A 层负向对照：多一行必须 different（行数不同）", () => {
  const more = [...SESSION_SAMPLE, '{"id":"<ID6>","parentId":"<ID5>","type":"message"}'];
  const result = compareSessionLines(withSession(SESSION_SAMPLE), withSession(more));
  assert.equal(result.status, STATUS.different);
  assert.match(result.detail, /行数不同/);
});

test("A 层负向对照：parentId 指错必须 different", () => {
  const target = SESSION_SAMPLE.findIndex((line) => line.includes('"parentId":"<ID2>"'));
  assert.ok(target >= 0, "样本里必须有 parentId=<ID2> 的条目，否则对照空转");
  const wrong = SESSION_SAMPLE.map((line, index) =>
    index === target ? line.replace('"parentId":"<ID2>"', '"parentId":"deadbeef"') : line,
  );
  const result = compareSessionLines(withSession(SESSION_SAMPLE), withSession(wrong));
  assert.equal(result.status, STATUS.different);
});

test("A 层负向对照：replacement 改值必须 different", () => {
  const target = SESSION_SAMPLE.findIndex((line) => line.includes('"type":"context_edit"'));
  assert.ok(target >= 0, "样本里必须有 context_edit 条目，否则对照空转（S01 的教训）");
  const changed = SESSION_SAMPLE.map((line, index) =>
    index === target ? line.replace('"replacement":null', '"replacement":{"x":1}') : line,
  );
  const result = compareSessionLines(withSession(SESSION_SAMPLE), withSession(changed));
  assert.equal(result.status, STATUS.different);
});

test("A 层：一侧缺 session.jsonl 必须 missing（不得当成 same）", () => {
  const result = compareSessionLines(withSession(SESSION_SAMPLE), null);
  assert.equal(result.status, STATUS.missing);
});

// ---------------------------------------------------------------------------
// A 层：`toolsAdded[*].parameters` **逐字节比**（不做键序归一化）。
//
// 理由：TS 的 `declarationsEqual`（`ref/.../ai/src/utils/transcript.ts:140-141`）
// 用 `JSON.stringify` 比对工具声明，**键序参与判等**。若比较器把它归一化掉，
// X02 那类"工具声明被重新排序 → TS 认为声明变了"的缺陷就会被静默放过。
// ---------------------------------------------------------------------------

const TOOL_DECL = (parametersText) =>
  `{"id":"<ID9>","type":"message","message":{"role":"system","content":"","timestamp":0,` +
  `"sections":{"cwd":"/tmp"},"toolsAdded":[{"name":"read","description":"d","parameters":${parametersText},` +
  `"constrainedSampling":{"type":"json_schema","strict":"prefer"}}]}}`;

test("A 层负向对照：toolsAdded.parameters 键序不同必须 different", () => {
  const tsOrder = TOOL_DECL('{"type":"object","required":["path"],"properties":{"path":{}}}');
  const sorted = TOOL_DECL('{"properties":{"path":{}},"required":["path"],"type":"object"}');
  // **对照必须真的触发**：样本里必须有 `toolsAdded` 与 `parameters`。
  assert.ok(tsOrder.includes('"toolsAdded"'), "样本必须含 toolsAdded");
  assert.ok(tsOrder.includes('"parameters"'), "样本必须含 parameters");
  const result = compareSessionLines(tsOrder + "\n", sorted + "\n");
  assert.equal(
    result.status,
    STATUS.different,
    "parameters 的键序不同必须判不同（不能被归一化掉）",
  );
});

test("A 层：parameters 之外的键序仍然不敏感（其余照旧归一化）", () => {
  const a = '{"id":"<ID1>","type":"session","cwd":"<cwd>","version":3}';
  const b = '{"version":3,"cwd":"<cwd>","id":"<ID1>","type":"session"}';
  const result = compareSessionLines(a + "\n", b + "\n");
  assert.equal(result.status, STATUS.same, "顶层键序不是判据");
});

test("A 层：parameters 键序相同仍判 same（防止过严）", () => {
  const same = TOOL_DECL('{"type":"object","required":["path"],"properties":{"path":{}}}');
  const result = compareSessionLines(same + "\n", same + "\n");
  assert.equal(result.status, STATUS.same);
});

// 负向对照（Q22）：两侧都没有真的跑起来时，**不得**判 same。
test("两侧产出都为空（有 FAILED.md / 0 字节文件）时不得判 same", async () => {
  const dir = await mkdtemp(path.join(tmpdir(), "cmp-gate-"));
  const golden = path.join(dir, "golden");
  const actual = path.join(dir, "actual");
  for (const side of [golden, actual]) {
    await mkdir(side, { recursive: true });
    await writeFile(path.join(side, "session.jsonl"), "");
    await writeFile(path.join(side, "events.jsonl"), "");
    await writeFile(path.join(side, "requests.jsonl"), "");
    await writeFile(path.join(side, "fs.json"), "{}\n");
    await writeFile(path.join(side, "FAILED.md"), "# failed\n");
    await writeFile(path.join(side, "meta.json"), '{"exitCode":1}\n');
  }
  const layers = await compareScenario(golden, actual);
  // 对照必须真的触发：两侧文件确实都存在但都是空的。
  assert.equal(fsStatSize(path.join(golden, "session.jsonl")).size, 0);
  assert.ok(fsExists(path.join(actual, "FAILED.md")));
  for (const name of ["A", "B", "C", "fs", "exit"]) {
    assert.notEqual(layers[name].status, "same", `${name} 层不得判 same：${JSON.stringify(layers[name])}`);
  }
  await rm(dir, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// Q-S25 规则（P4-15 步骤 3）：**只在 anthropic 场景**、**只在 `message_start`** 里，
// 忽略 `message.content` 中 `type === "text" && text === ""` 的块。
// 依据：`golden/_runs/S25-anthropic` 三次录制里该块时有时无（`notes/p413-step1-s25-rerecord.md`）。
// ---------------------------------------------------------------------------

const EMPTY_BLOCK_EVENT = (provider) =>
  `{"type":"message_start","message":{"role":"assistant","content":[{"type":"text","text":"","index":0}],"provider":"mock"},"x":1}\n`;
const NO_BLOCK_EVENT =
  '{"type":"message_start","message":{"role":"assistant","content":[],"provider":"mock"},"x":1}\n';

/** 造一对"只有 events.jsonl 不同"的产出，provider 由 `meta.json` 决定规则是否启用。 */
async function makeAnthropicPair({ goldenEvents, actualEvents, provider = "anthropic-messages" }) {
  const meta = `{"exitCode":0,"provider":${JSON.stringify(provider)}}\n`;
  const pair = await makePair({
    "events.jsonl": goldenEvents,
    "requests.jsonl": "{}\n",
    "fs.json": "{}\n",
    "meta.json": meta,
  });
  await writeFile(path.join(pair.actualDir, "events.jsonl"), actualEvents);
  return pair;
}

test("Q-S25：anthropic 的 message_start 空串 text 块差异判 same", async () => {
  const pair = await makeAnthropicPair({
    goldenEvents: EMPTY_BLOCK_EVENT("anthropic-messages"),
    actualEvents: NO_BLOCK_EVENT,
  });
  // 样本自检（防止空转）：两侧都确实有 message_start，且**原文不同**（一个带空块、一个不带）。
  const goldenText = await readFile(path.join(pair.goldenDir, "events.jsonl"), "utf8");
  const actualText = await readFile(path.join(pair.actualDir, "events.jsonl"), "utf8");
  assert.ok(goldenText.includes('"type":"message_start"'), "golden 样本含目标事件");
  assert.ok(actualText.includes('"type":"message_start"'), "actual 样本含目标事件");
  assert.ok(goldenText.includes('"text":""'), "golden 样本确实带空串 text 块");
  assert.ok(!actualText.includes('"text":""'), "actual 样本确实不带该块");
  assert.notEqual(goldenText, actualText, "两侧原文必须真的不同");

  const layers = await compareScenario(pair.goldenDir, pair.actualDir);
  assert.equal(layers.B.status, STATUS.same, "空串 text 块差异应被忽略");
  await rm(pair.root, { recursive: true, force: true });
});

test("Q-S25 负向：非空 text 块差异必须 different", async () => {
  const pair = await makeAnthropicPair({
    goldenEvents:
      '{"type":"message_start","message":{"role":"assistant","content":[{"type":"text","text":"Hi","index":0}]},"x":1}\n',
    actualEvents: NO_BLOCK_EVENT,
  });
  const layers = await compareScenario(pair.goldenDir, pair.actualDir);
  assert.equal(layers.B.status, STATUS.different, "非空 text 块不能被忽略");
  await rm(pair.root, { recursive: true, force: true });
});

test("Q-S25 负向：message_start 的其它字段差异必须 different", async () => {
  const pair = await makeAnthropicPair({
    goldenEvents: NO_BLOCK_EVENT,
    actualEvents:
      '{"type":"message_start","message":{"role":"assistant","content":[],"provider":"other"},"x":1}\n',
  });
  const layers = await compareScenario(pair.goldenDir, pair.actualDir);
  assert.equal(layers.B.status, STATUS.different, "只有空串 text 块才放行");
  await rm(pair.root, { recursive: true, force: true });
});

test("Q-S25 负向：空串 text 块规则不适用于 openai 场景", async () => {
  const pair = await makeAnthropicPair({
    goldenEvents: EMPTY_BLOCK_EVENT("openai-completions"),
    actualEvents: NO_BLOCK_EVENT,
    provider: "openai-completions",
  });
  const layers = await compareScenario(pair.goldenDir, pair.actualDir);
  assert.equal(layers.B.status, STATUS.different, "规则必须限定在 anthropic 场景");
  await rm(pair.root, { recursive: true, force: true });
});

test("Q-S25 负向：规则不碰 message_update 里的空串 text 块", async () => {
  const update = (text) =>
    `{"type":"message_update","assistantMessageEvent":{"type":"text_delta","text":${JSON.stringify(text)}},"x":1}\n`;
  const pair = await makeAnthropicPair({ goldenEvents: update(""), actualEvents: update("Hi") });
  const layers = await compareScenario(pair.goldenDir, pair.actualDir);
  assert.equal(layers.B.status, STATUS.different, "只处理 message_start，不碰 message_update");
  await rm(pair.root, { recursive: true, force: true });
});
