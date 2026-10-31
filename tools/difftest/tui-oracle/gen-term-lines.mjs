// P7-6w/W2：把 `fixtures-term.json` 转成 **Rust 单测可读的文本侧车** `fixtures-term.lines.txt`。
//
// 为什么要这一步：`pi-tui` 不允许引入 JSON 解析依赖（批次约束：不新增依赖）。
// 本脚本**不改** `gen-term.mjs`，而是**重新声明同一张用例表**（输入步骤），用 ref 的真实
// `ProcessTerminal` 重跑，并**断言重跑结果与 `fixtures-term.json` 完全一致**（防止两份表漂移）；
// 一致后才写侧车。
//
// 侧车格式（每行一条）：
//   case <id> <步骤数>
//   step <name> <chunkBase64>[,<chunkBase64>...] <waitMs>
//   kitty <at> <0|1>
//   out <phaseName> <bytesBase64>
//   event <bytesBase64>
import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";

const here = import.meta.dirname;
const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { ProcessTerminal } = await import(path.join(REF, "terminal.ts"));
const keys = await import(path.join(REF, "keys.ts"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const b64 = (s) => Buffer.from(String(s), "utf8").toString("base64");

// 与 gen-term.mjs 等价的用例表（输入步骤；输出由真实类重跑得到并校验）
const TABLE = [
	["n1-no-response-key", [{ feed: ["a"], name: "key" }]],
	["n2-no-response-arrow", [{ feed: ["\x1b[A"], name: "arrow" }]],
	["n3-no-response-paste", [{ feed: ["\x1b[200~hi\x1b[201~"], name: "paste" }]],
	["k1-kitty-flags1", [{ feed: ["\x1b[?1u"], waitMs: 200, name: "kitty" }]],
	["k2-kitty-flags0", [{ feed: ["\x1b[?0u"], waitMs: 200, name: "flags0" }]],
	["k3-kitty-flags5-then-key", [{ feed: ["\x1b[?5u"], waitMs: 200, name: "kitty5" }, { feed: ["\x1b[97;5u"], name: "key" }]],
	["d1-da1-only", [{ feed: ["\x1b[?1;2c"], waitMs: 200, name: "da1" }]],
	["d2-da1-then-key", [{ feed: ["\x1b[?1;2c"], waitMs: 200, name: "da1" }, { feed: ["x"], name: "key" }]],
	["d3-da1-after-kitty", [{ feed: ["\x1b[?1u"], waitMs: 200, name: "kitty" }, { feed: ["\x1b[?1;2c"], waitMs: 200, name: "da1" }]],
	["o1-osc11-bel", [{ feed: ["\x1b]11;rgb:0000/0000/0000\x07"], waitMs: 200, name: "osc11" }]],
	["o2-osc11-st", [{ feed: ["\x1b]11;rgb:ffff/ffff/ffff\x1b\\"], waitMs: 200, name: "osc11st" }]],
	["o3-osc11-then-key", [{ feed: ["\x1b]11;rgb:1111/2222/3333\x07"], waitMs: 200, name: "osc11" }, { feed: ["k"], name: "key" }]],
	["f1-kitty-split", [{ feed: ["\x1b[?", "1u"], waitMs: 200, name: "split" }]],
	["f2-da1-split", [{ feed: ["\x1b[?1", ";2c"], waitMs: 200, name: "split" }]],
	["f3-kitty-split-3", [{ feed: ["\x1b", "[?1", "u"], waitMs: 200, name: "split3" }]],
	["t1-late-tail", [{ feed: ["\x1b[?"], waitMs: 250, name: "half" }, { feed: ["1u"], waitMs: 200, name: "tail" }]],
	["t2-late-da1", [{ feed: ["\x1b[?1"], waitMs: 250, name: "half" }, { feed: [";2c"], waitMs: 200, name: "tail" }]],
	["t3-late-after-key", [{ feed: ["\x1b[?"], waitMs: 250, name: "half" }, { feed: ["z"], name: "key" }, { feed: ["1u"], waitMs: 200, name: "tail" }]],
	["i1-key-then-kitty", [{ feed: ["a\x1b[?1u"], waitMs: 200, name: "mixed" }]],
	["i2-kitty-then-key", [{ feed: ["\x1b[?1ub"], waitMs: 200, name: "mixed" }]],
	["i3-key-kitty-key", [{ feed: ["c"], name: "k1" }, { feed: ["\x1b[?1u"], waitMs: 200, name: "kitty" }, { feed: ["d"], name: "k2" }]],
	["s1-stop-after-kitty", [{ feed: ["\x1b[?1u"], waitMs: 200, name: "kitty" }]],
	["s2-stop-after-flags0", [{ feed: ["\x1b[?0u"], waitMs: 200, name: "flags0" }]],
	["s3-stop-plain", []],
];

async function runCase(id, steps) {
	const written = [];
	const events = [];
	const kittyTimeline = [];
	const fakeOut = { write: (s) => { written.push(String(s)); return true; }, isTTY: false, columns: 100, rows: 30, on: () => {}, removeListener: () => {} };
	const fakeIn = Object.assign(new EventEmitter(), {
		isTTY: true, isRaw: false, setRawMode() { return this; }, setEncoding() { return this; }, resume() { return this; }, pause() { return this; },
	});
	const origOut = process.stdout, origIn = process.stdin;
	Object.defineProperty(process, "stdout", { value: fakeOut, configurable: true });
	Object.defineProperty(process, "stdin", { value: fakeIn, configurable: true });
	const phases = [];
	const mark = (name) => phases.push({ name, bytes: written.splice(0).join("") });
	try {
		const term = new ProcessTerminal();
		term.start((data) => { events.push(data); kittyTimeline.push({ at: `event:${data.length}`, kitty: keys.isKittyProtocolActive() }); }, () => {});
		mark("start");
		for (const step of steps) {
			if (step.feed !== undefined) for (const chunk of step.feed) { fakeIn.emit("data", chunk); await sleep(step.waitAfterChunkMs ?? 5); }
			if (step.waitMs) await sleep(step.waitMs);
			mark(step.name ?? "step");
			kittyTimeline.push({ at: step.name ?? "step", kitty: keys.isKittyProtocolActive() });
		}
		term.stop();
		mark("stop");
	} finally {
		Object.defineProperty(process, "stdout", { value: origOut, configurable: true });
		Object.defineProperty(process, "stdin", { value: origIn, configurable: true });
	}
	return { id, phases, kittyTimeline, events, leftover: events.join("") };
}

const fixturePath = path.join(here, "fixtures-term.json");
const expected = JSON.parse(fs.readFileSync(fixturePath, "utf8")).cases;
const actual = [];
for (const [id, steps] of TABLE) actual.push(await runCase(id, steps));

// 校验：重跑结果必须与入库夹具完全一致（防两表漂移）
let mismatch = 0;
for (let i = 0; i < Math.max(expected.length, actual.length); i += 1) {
	const a = actual[i], e = expected[i];
	if (!a || !e || JSON.stringify(a) !== JSON.stringify(e)) {
		mismatch += 1;
		console.error(`不一致: 期望 ${e?.id} 实际 ${a?.id}`);
	}
}
if (mismatch > 0) { console.error(`校验失败：${mismatch}/${expected.length}`); process.exit(1); }
console.log(`校验通过：${actual.length}/${expected.length} 与 fixtures-term.json 完全一致`);

const lines = [];
for (let i = 0; i < TABLE.length; i += 1) {
	const [id, steps] = TABLE[i];
	lines.push(`case ${id} ${steps.length}`);
	for (const step of steps) {
		const chunks = (step.feed ?? []).map(b64).join(",");
		const wait = step.waitMs ?? 0;
		const perChunkWait = step.waitAfterChunkMs ?? 5;
		lines.push(`step ${step.name ?? "step"} ${chunks || "-"} ${wait} ${perChunkWait}`);
	}
	for (const k of actual[i].kittyTimeline) lines.push(`kitty ${k.at} ${k.kitty ? 1 : 0}`);
	for (const phase of actual[i].phases) lines.push(`out ${phase.name} ${b64(phase.bytes)}`);
	for (const ev of actual[i].events) lines.push(`event ${b64(ev)}`);
}
const outPath = path.join(here, "fixtures-term.lines.txt");
fs.writeFileSync(outPath, lines.join("\n") + "\n");
console.log(`侧车已写：${outPath}（${lines.length} 行，${(fs.statSync(outPath).size / 1024).toFixed(1)} KB）`);
