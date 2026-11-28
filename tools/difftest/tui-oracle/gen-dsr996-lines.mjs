// P7-6af/D：把 `fixtures-dsr996.json` 转成 Rust 可读的文本侧车（`fixtures-dsr996.lines.txt`）。
//
// 不改 `gen-dsr996.mjs`：重声明同一张输入表，用 ref 真实 `TuiMainScreen` 重跑并**断言与 JSON 一致**后写侧车。
import fs from "node:fs";
import path from "node:path";
const here = import.meta.dirname;
const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { TuiMainScreen } = await import(path.join(REF, "tui-main-screen.ts"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const b64 = (s) => Buffer.from(String(s), "utf8").toString("base64");

const TABLE = [
	["n1-dark", 120, ["\x1b[?997;1n"]],
	["n2-light", 120, ["\x1b[?997;2n"]],
	["n3-two-reports", 120, ["\x1b[?997;1n\x1b[?997;2n"]],
	["f1-split-2", 120, ["\x1b[?997", ";1n"]],
	["f2-split-3", 120, ["\x1b[?9", "97;", "2n"]],
	["f3-split-tail", 120, ["\x1b[?997;1", "n"]],
	["i1-between-keys", 120, ["a", "\x1b[?997;1n", "b"]],
	["i2-key-then-report", 120, ["x\x1b[?997;2n"]],
	["i3-report-then-key", 120, ["\x1b[?997;1ny"]],
	["t1-timeout", 80, []],
	["t2-timeout-then-report", 80, ["\x1b[?997;1n"]],
	["t3-timeout-then-key", 80, ["q"]],
	["d1-repeat", 120, ["\x1b[?997;1n\x1b[?997;1n\x1b[?997;2n"]],
	["d2-bad-scheme", 80, ["\x1b[?997;3n"]],
	["d3-not-997", 80, ["\x1b[?996;1n"]],
];

async function run(id, timeoutMs, chunks) {
	const seen = [];
	const writes = [];
	const fake = { write(s) { writes.push(String(s)); return true; }, start(onInput) { this.onInput = onInput; }, stop() {}, hideCursor() {}, showCursor() {}, get columns() { return 100; }, get rows() { return 30; }, on() {}, off() {}, removeListener() {} };
	const screen = new TuiMainScreen(fake);
	screen.start();
	screen.addInputListener((d) => { seen.push(d); return undefined; });
	const p = screen.queryTerminalColorScheme({ timeoutMs });
	for (const c of chunks) { fake.onInput(c); await sleep(2); }
	const scheme = (await p) ?? null;
	await sleep(5);
	return { id, scheme, events: seen.slice(), query: writes.filter((s) => s.includes("?996n")).join("") };
}

const expected = JSON.parse(fs.readFileSync(path.join(here, "fixtures-dsr996.json"), "utf8")).cases;
const actual = [];
for (const [id, t, chunks] of TABLE) actual.push(await run(id, t, chunks));
let bad = 0;
for (let i = 0; i < expected.length; i += 1) {
	if (expected[i].id !== actual[i].id || JSON.stringify(expected[i].scheme) !== JSON.stringify(actual[i].scheme)
		|| JSON.stringify(expected[i].componentEvents) !== JSON.stringify(actual[i].events)) {
		bad += 1;
		console.error(`不一致 ${expected[i].id}`);
	}
}
if (bad > 0) { console.error(`校验失败 ${bad}/${expected.length}`); process.exit(1); }
console.log(`校验通过：${actual.length}/${expected.length} 与 fixtures-dsr996.json 一致`);

const lines = [];
for (let i = 0; i < TABLE.length; i += 1) {
	const [id, , chunks] = TABLE[i];
	lines.push(`case ${id}`);
	for (const c of chunks) lines.push(`chunk ${b64(c)}`);
	lines.push(actual[i].scheme ? `scheme ${actual[i].scheme}` : "scheme -");
	for (const e of actual[i].events) lines.push(`event ${b64(e)}`);
}
const side = path.join(here, "fixtures-dsr996.lines.txt");
fs.writeFileSync(side, lines.join("\n") + "\n");
console.log(`侧车已写：${side}（${lines.length} 行）`);
