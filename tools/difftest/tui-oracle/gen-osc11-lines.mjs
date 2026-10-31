// P7-6y/W3：把 `fixtures-osc11.json` 转成 Rust 单测可读的**文本侧车**（`fixtures-osc11.lines.txt`）。
//
// 与 `gen-term-lines.mjs` 同一套路：不改 `gen-osc11.mjs`，而是**重新声明同一张用例表**（含输入字节），
// 用 ref 真实 `TuiMainScreen` + 假终端重跑，断言与 `fixtures-osc11.json` **完全一致**后才写侧车。
//
// 侧车格式：
//   case <id> <queryCount>
//   chunk <base64>
//   parsed <0|1> <r> <g> <b>      （每次 query 的解析结果；1=解析出 RGB）
//   event <base64>                （过滤后仍进入组件层的事件）
import fs from "node:fs";
import path from "node:path";

const here = import.meta.dirname;
const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { TuiMainScreen } = await import(path.join(REF, "tui-main-screen.ts"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const TABLE = [
	["a1-normal-bel", 120, ["\x1b]11;rgb:1111/2222/3333\x07"]],
	["a2-normal-st", 120, ["\x1b]11;rgb:ffff/ffff/ffff\x1b\\"]],
	["a3-normal-black", 120, ["\x1b]11;rgb:0000/0000/0000\x07"]],
	["a4-split-2", 120, ["\x1b]11;rgb:1111/", "2222/3333\x07"]],
	["a5-split-3", 120, ["\x1b]11;", "rgb:2222/", "3333/4444\x07"]],
	["a6-split-tail", 120, ["\x1b]11;rgb:121", "2/3434/5656\x07"]],
	["a7-between-keys", 120, ["a", "\x1b]11;rgb:1111/2222/3333\x07", "b"]],
	["a8-before-key", 120, ["\x1b]11;rgb:4444/5555/6666\x07", "x"]],
	["a9-key-then-answer", 120, ["z", "\x1b]11;rgb:7777/8888/9999\x07"]],
	["b1-timeout", 80, []],
	["b2-timeout-then-answer", 80, ["\x1b]11;rgb:1111/2222/3333\x07"]],
	["b3-timeout-then-key", 80, ["q"]],
	["c1-double", 120, ["\x1b]11;rgb:1111/1111/1111\x07", "\x1b]11;rgb:2222/2222/2222\x07"]],
	["c2-two-queries", 120, ["\x1b]11;rgb:3333/3333/3333\x07", "\x1b]11;rgb:4444/4444/4444\x07"]],
	["c3-unsolicited", 120, ["\x1b]11;rgb:5555/5555/5555\x07"]],
	["d1-garbage", 80, ["\x1b]11;not-a-color\x07"]],
	["d2-short", 80, ["\x1b]11;rgb:11\x07"]],
	["d3-not-osc11", 80, ["\x1b]10;rgb:1111/2222/3333\x07"]],
];
// 需要发几次查询（与 gen-osc11.mjs 的步骤表一致；`c3-unsolicited` 不发查询）
const QUERIES = { "c2-two-queries": 2, "c3-unsolicited": 0 };

async function runCase(id, timeoutMs, chunks) {
	const writes = [];
	const seen = [];
	const fake = {
		write(s) { writes.push(String(s)); return true; },
		start(onInput) { this.onInput = onInput; },
		stop() {}, hideCursor() {}, showCursor() {},
		get columns() { return 100; }, get rows() { return 30; },
		on() {}, off() {}, removeListener() {},
	};
	const screen = new TuiMainScreen(fake);
	screen.start();
	screen.addInputListener((data) => { seen.push(data); return undefined; });
	const queries = QUERIES[id] ?? 1;
	const parsed = [];
	let pendingPromise = null;
	const issueQuery = () => { pendingPromise = screen.queryTerminalBackgroundColor({ timeoutMs }); };
	const awaitQuery = async () => { parsed.push(pendingPromise ? (await pendingPromise) ?? null : null); };
	if (id === "b2-timeout-then-answer") {
		issueQuery();
		await sleep(timeoutMs + 40);
		awaitQuery();
		fake.onInput(chunks[0]);
		await sleep(20);
	} else if (id === "c2-two-queries") {
		issueQuery();
		fake.onInput(chunks[0]);
		await sleep(5);
		awaitQuery();
		issueQuery();
		fake.onInput(chunks[1]);
		await sleep(5);
		awaitQuery();
	} else {
		if (queries > 0) issueQuery();
		for (const chunk of chunks) { fake.onInput(chunk); await sleep(2); }
		await sleep(timeoutMs + 40);
		if (queries > 0) awaitQuery();
	}
	const negotiated = writes.filter((s) => s.includes("]11;?")).join("");
	return { id, queryCount: queries, negotiatedWrites: negotiated, parsed, componentEvents: seen.slice() };
}

const fixture = JSON.parse(fs.readFileSync(path.join(here, "fixtures-osc11.json"), "utf8")).cases;
const actual = [];
for (const [id, timeoutMs, chunks] of TABLE) actual.push(await runCase(id, timeoutMs, chunks));

let mismatch = 0;
for (let i = 0; i < fixture.length; i += 1) {
	const a = actual[i], e = fixture[i];
	if (!a || a.id !== e.id || JSON.stringify(a.parsed) !== JSON.stringify(e.parsed)
		|| JSON.stringify(a.componentEvents) !== JSON.stringify(e.componentEvents)) {
		mismatch += 1;
		console.error(`不一致 ${e?.id}: 期望 parsed=${JSON.stringify(e?.parsed)} events=${JSON.stringify(e?.componentEvents)} 实际 parsed=${JSON.stringify(a?.parsed)} events=${JSON.stringify(a?.componentEvents)}`);
	}
}
if (mismatch > 0) { console.error(`校验失败 ${mismatch}/${fixture.length}`); process.exit(1); }
console.log(`校验通过：${actual.length}/${fixture.length} 与 fixtures-osc11.json 一致`);

const lines = [];
for (let i = 0; i < TABLE.length; i += 1) {
	const [id, , chunks] = TABLE[i];
	lines.push(`case ${id} ${actual[i].queryCount}`);
	for (const c of chunks) lines.push(`chunk ${Buffer.from(c, "utf8").toString("base64")}`);
	for (const p of actual[i].parsed) {
		lines.push(p ? `parsed 1 ${p.r} ${p.g} ${p.b}` : "parsed 0 0 0 0");
	}
	for (const ev of actual[i].componentEvents) lines.push(`event ${Buffer.from(ev, "utf8").toString("base64")}`);
}
const side = path.join(here, "fixtures-osc11.lines.txt");
fs.writeFileSync(side, lines.join("\n") + "\n");
console.log(`侧车已写：${side}（${lines.length} 行，${(fs.statSync(side).size / 1024).toFixed(1)} KB）`);
