// P7-6x/W3：**OSC 11 / DSR 996 应答消费 oracle**（假终端，实例化 ref 真实 `TuiMainScreen`）。
//
// 不依赖真实 TTY：假终端对象提供 `write`(捕获) / `start`(捕获 inputHandler) / 尺寸 getter；
// 通过 `addInputListener` 观察"过滤之后仍会进入组件层的事件"，以此判定应答是否被消费。
//
// 时间：查询超时用 `queryTerminalBackgroundColor({ timeoutMs })` 的显式参数 + 手动 `await`，
// 不依赖真实时钟精度（每例都用固定 timeoutMs 与小步 sleep）。
import fs from "node:fs";
import path from "node:path";

const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { TuiMainScreen } = await import(path.join(REF, "tui-main-screen.ts"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function runCase(id, steps, { timeoutMs = 120 } = {}) {
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
	const phases = [];
	// 只记录**协商相关**的写出（OSC 11 查询 / DSR 996 / 彩色方案通知开关）；
	// 渲染循环的同步更新帧（`\x1b[?2026h` 等）随调度变化，属噪声，统计到 `frameWrites` 计数后丢弃。
	let frameWrites = 0;
	const relevant = (s) => s.includes("]11;?") || s.includes("?996n") || s.includes("?2031") || s.includes("]11;");
	// 只累积"协商相关写出"的总串（**不按阶段切分**：阶段边界会与渲染调度竞争，导致非确定性）
	const negotiated = [];
	const mark = (name) => {
		const all = writes.splice(0);
		for (const s of all) if (relevant(s)) negotiated.push(s);
		frameWrites += all.filter((s) => !relevant(s)).length;
		phases.push({ name });
	};
	screen.start();
	screen.addInputListener((data) => { seen.push(data); return undefined; });
	mark("start");
	const results = [];
	let pendingPromise = null;
	for (const step of steps) {
		if (step.query) {
			pendingPromise = screen.queryTerminalBackgroundColor({ timeoutMs });
			mark(step.name ?? "query");
		}
		for (const chunk of step.feed ?? []) {
			fake.onInput(chunk);
			await sleep(2);
		}
		if (step.awaitQuery) {
			const value = pendingPromise ? await pendingPromise : null;
			results.push(value ?? null);
			mark(step.name ?? "answer");
		}
		if (step.waitMs) await sleep(step.waitMs);
	}
	mark("end");
	screen.stop?.();
	return { id, negotiationWrites: negotiated.join(""), parsed: results, componentEvents: seen.slice(), leftover: seen.join("") };
}

const cases = [];
const add = async (id, steps, opts) => cases.push(await runCase(id, steps, opts));

// ① 正常应答（BEL 与 ST 两种结尾）
await add("a1-normal-bel", [{ query: true, name: "query" }, { feed: ["\x1b]11;rgb:1111/2222/3333\x07"], awaitQuery: true, name: "bel" }]);
await add("a2-normal-st", [{ query: true }, { feed: ["\x1b]11;rgb:ffff/ffff/ffff\x1b\\"], awaitQuery: true, name: "st" }]);
await add("a3-normal-black", [{ query: true }, { feed: ["\x1b]11;rgb:0000/0000/0000\x07"], awaitQuery: true, name: "black" }]);
// ② 应答被分片
await add("a4-split-2", [{ query: true }, { feed: ["\x1b]11;rgb:1111/", "2222/3333\x07"], awaitQuery: true, name: "split2" }]);
await add("a5-split-3", [{ query: true }, { feed: ["\x1b]11;", "rgb:2222/", "3333/4444\x07"], awaitQuery: true, name: "split3" }]);
await add("a6-split-tail", [{ query: true }, { feed: ["\x1b]11;rgb:1212/3434/5656\x07".slice(0, 12), "\x1b]11;rgb:1212/3434/5656\x07".slice(12)], awaitQuery: true, name: "splitTail" }]);
// ③ 应答夹在按键之间
await add("a7-between-keys", [{ query: true }, { feed: ["a"] }, { feed: ["\x1b]11;rgb:1111/2222/3333\x07"], awaitQuery: true }, { feed: ["b"], name: "after" }]);
await add("a8-before-key", [{ query: true }, { feed: ["\x1b]11;rgb:4444/5555/6666\x07"], awaitQuery: true, name: "answer" }, { feed: ["x"], name: "key" }]);
await add("a9-key-then-answer", [{ query: true }, { feed: ["z"] }, { feed: ["\x1b]11;rgb:7777/8888/9999\x07"], awaitQuery: true, name: "answer" }]);
// ④ 超时无应答
await add("b1-timeout", [{ query: true, name: "query" }, { waitMs: 200, awaitQuery: true, name: "timeout" }], { timeoutMs: 80 });
await add("b2-timeout-then-answer", [{ query: true }, { waitMs: 200, awaitQuery: true, name: "timeout" }, { feed: ["\x1b]11;rgb:1111/2222/3333\x07"], waitMs: 20, name: "late" }], { timeoutMs: 80 });
await add("b3-timeout-then-key", [{ query: true }, { waitMs: 200, awaitQuery: true, name: "timeout" }, { feed: ["q"], name: "key" }], { timeoutMs: 80 });
// ⑤ 重复应答
await add("c1-double", [{ query: true }, { feed: ["\x1b]11;rgb:1111/1111/1111\x07"], awaitQuery: true }, { feed: ["\x1b]11;rgb:2222/2222/2222\x07"], waitMs: 20, name: "second" }]);
await add("c2-two-queries", [{ query: true }, { feed: ["\x1b]11;rgb:3333/3333/3333\x07"], awaitQuery: true }, { query: true }, { feed: ["\x1b]11;rgb:4444/4444/4444\x07"], awaitQuery: true, name: "second" }]);
await add("c3-unsolicited", [{ feed: ["\x1b]11;rgb:5555/5555/5555\x07"], waitMs: 20, name: "noQuery" }]);
// ⑥ 非法格式
await add("d1-garbage", [{ query: true }, { feed: ["\x1b]11;not-a-color\x07"], awaitQuery: true, name: "garbage" }], { timeoutMs: 80 });
await add("d2-short", [{ query: true }, { feed: ["\x1b]11;rgb:11\x07"], awaitQuery: true, name: "short" }], { timeoutMs: 80 });
await add("d3-not-osc11", [{ query: true }, { feed: ["\x1b]10;rgb:1111/2222/3333\x07"], awaitQuery: true, name: "osc10" }], { timeoutMs: 80 });

const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-osc11.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-osc11.mjs", tsVersion: "0.87.1", cases }, null, 0));
console.error(`OSC 11 夹具：用例=${cases.length} 大小=${(fs.statSync(out).size / 1024).toFixed(1)} KB 输出=${out}`);
