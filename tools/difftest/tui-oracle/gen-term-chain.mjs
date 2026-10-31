// P7-6y/W2：**链路夹具**（字节 → StdinBuffer → 协商层 → 输入处理器）。
//
// 用 ref 的真实 `ProcessTerminal`（内含真实 `StdinBuffer` 与协商逻辑）跑"按字节分片喂入"的用例，
// 记录：①输入字节分片（按序）②每片之后**两个定时器都已到期**（等待 200ms > 150ms 聚合 + > 10ms ESC 超时）
// ③最终交给输入处理器的事件序列。Rust 侧 `tests/term_chain.rs` 用同一份字节驱动
// `input.rs`（StdinBuffer 端口）→ `term_negotiate`，逐例比对事件序列。
import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";

const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { ProcessTerminal } = await import(path.join(REF, "terminal.ts"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const b64 = (s) => Buffer.from(String(s), "utf8").toString("base64");

/** 每片之后等待 200ms：让 StdinBuffer 的 ESC/序列超时与协商层的 150ms 都必然到期。 */
const SETTLE_MS = 200;

async function runCase(id, chunks) {
	const events = [];
	const fakeOut = { write: () => true, isTTY: false, columns: 100, rows: 30, on: () => {}, removeListener: () => {} };
	const fakeIn = Object.assign(new EventEmitter(), {
		isTTY: true, isRaw: false, setRawMode() { return this; }, setEncoding() { return this; }, resume() { return this; }, pause() { return this; },
	});
	const origOut = process.stdout, origIn = process.stdin;
	Object.defineProperty(process, "stdout", { value: fakeOut, configurable: true });
	Object.defineProperty(process, "stdin", { value: fakeIn, configurable: true });
	try {
		const term = new ProcessTerminal();
		term.start((data) => events.push(data), () => {});
		for (const chunk of chunks) { fakeIn.emit("data", chunk); await sleep(SETTLE_MS); }
		term.stop();
	} finally {
		Object.defineProperty(process, "stdout", { value: origOut, configurable: true });
		Object.defineProperty(process, "stdin", { value: origIn, configurable: true });
	}
	return { id, chunks: chunks.map(b64), events: events.map(b64) };
}

const cases = [];
const add = async (id, chunks) => cases.push(await runCase(id, chunks));

// A. 应答被分片切开（3 例）
await add("c1-split-kitty", ["\x1b[?", "1u"]);
await add("c2-split-da1", ["\x1b[?1", ";2c"]);
await add("c3-split-three", ["\x1b", "[?1", "u"]);
// B. 应答夹在按键之间（3 例）
await add("c4-key-resp-key", ["a", "\x1b[?1u", "b"]);
await add("c5-resp-between", ["x\x1b[?3u", "y"]);
await add("c6-two-resp-between", ["a", "\x1b[?1u", "b", "\x1b[?1;2c", "c"]);
// C. 孤立 ESC 与协商应答并存（3 例）
await add("c7-esc-then-resp", ["\x1b", "\x1b[?1u"]);
await add("c8-esc-arrow-resp", ["\x1b", "[A", "\x1b[?1u"]);
await add("c9-resp-then-esc", ["\x1b[?1u", "\x1b"]);
// D. 括号粘贴内含应答样字节（3 例）
await add("c10-paste-osc11", ["\x1b[200~x\x1b]11;rgb:1111/2222/3333\x07y\x1b[201~"]);
await add("c11-paste-kitty", ["\x1b[200~p\x1b[?1uq\x1b[201~"]);
await add("c12-paste-split", ["\x1b[200~he", "llo\x1b[201~"]);

const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-term-chain.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-term-chain.mjs", tsVersion: "0.87.1", settleMs: SETTLE_MS, cases }, null, 0));
const lines = [];
for (const c of cases) {
	lines.push(`case ${c.id} ${c.chunks.length}`);
	for (const ch of c.chunks) lines.push(`chunk ${ch}`);
	for (const ev of c.events) lines.push(`event ${ev}`);
}
const side = out.replace(/\.json$/, ".lines.txt");
fs.writeFileSync(side, lines.join("\n") + "\n");
console.error(`链路夹具：用例=${cases.length} 大小=${(fs.statSync(out).size / 1024).toFixed(1)} KB 侧车=${(fs.statSync(side).size / 1024).toFixed(1)} KB`);
