// P7-6ae/S7：**DSR 996 / 997 应答消费 oracle**（假终端，实例化 ref 真实 `TuiMainScreen`）。
//
// 规格：查询 `\x1b[?996n`（`tui.ts:1453`）；应答 `\x1b[?997;1n`（dark）/ `\x1b[?997;2n`（light）；
// 消费点 `tui.ts:1010` → `consumeTerminalColorSchemeReport`（`:1108-1118`）→ `parseTerminalColorSchemeReport`
// （`terminal-colors.ts:67-73`，正则 `^(?:\x1b\[\?997;(1|2)n)+$`，`:29`）。
import fs from "node:fs";
import path from "node:path";

const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { TuiMainScreen } = await import(path.join(REF, "tui-main-screen.ts"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function runCase(id, chunks, { timeoutMs = 120 } = {}) {
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
	screen.addInputListener((d) => { seen.push(d); return undefined; });
	const promise = screen.queryTerminalColorScheme({ timeoutMs });
	for (const c of chunks) { fake.onInput(c); await sleep(2); }
	const scheme = (await promise) ?? null;
	await sleep(10);
	return {
		id,
		negotiationWrites: writes.filter((s) => s.includes("?996n") || s.includes("997")).join(""),
		scheme,
		componentEvents: seen.slice(),
	};
}

const cases = [];
const add = async (id, chunks, opts) => cases.push(await runCase(id, chunks, opts));
// ① 正常应答（dark / light / 连续两条）
await add("n1-dark", ["\x1b[?997;1n"]);
await add("n2-light", ["\x1b[?997;2n"]);
await add("n3-two-reports", ["\x1b[?997;1n\x1b[?997;2n"]);
// ② 分片
await add("f1-split-2", ["\x1b[?997", ";1n"]);
await add("f2-split-3", ["\x1b[?9", "97;", "2n"]);
await add("f3-split-tail", ["\x1b[?997;1", "n"]);
// ③ 夹在按键间
await add("i1-between-keys", ["a", "\x1b[?997;1n", "b"]);
await add("i2-key-then-report", ["x\x1b[?997;2n"]);
await add("i3-report-then-key", ["\x1b[?997;1ny"]);
// ④ 无应答（超时）
await add("t1-timeout", [], { timeoutMs: 80 });
await add("t2-timeout-then-report", ["\x1b[?997;1n"], { timeoutMs: 80 });
await add("t3-timeout-then-key", ["q"], { timeoutMs: 80 });
// ⑤ 重复应答 / 非法格式
await add("d1-repeat", ["\x1b[?997;1n\x1b[?997;1n\x1b[?997;2n"]);
await add("d2-bad-scheme", ["\x1b[?997;3n"], { timeoutMs: 80 });
await add("d3-not-997", ["\x1b[?996;1n"], { timeoutMs: 80 });

const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-dsr996.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-dsr996.mjs", tsVersion: "0.87.1", cases }, null, 0));
console.error(`DSR996 夹具：用例=${cases.length} 大小=${(fs.statSync(out).size / 1024).toFixed(1)} KB`);
