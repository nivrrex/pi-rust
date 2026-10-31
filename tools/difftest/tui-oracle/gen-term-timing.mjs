// P7-6af/A.2：**协商层转发时机 oracle**——记录"序列被转发发生在哪一步/是否要等 150 ms 超时"。
//
// 盲区来源（A.1）：`term_fixtures.rs:186` 与 `term_chain.rs:115` 每步都无条件调 `on_timeout()`，
// 而断言只比"最终事件序列"，于是"立即转发"与"等 150 ms 后转发"看起来完全一样。
//
// 做法：用 ref 真实 `ProcessTerminal`（内含真实 StdinBuffer + 协商逻辑），逐步喂入并在**每步之后**
// 记录已经交给输入处理器的事件；步与步之间**不等待**（0 ms），只有显式 `wait` 步才等 200 ms
// （> 150 ms 聚合超时）并记录"超时之后新增的事件"。
import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";

const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { ProcessTerminal } = await import(path.join(REF, "terminal.ts"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const b64 = (s) => Buffer.from(String(s), "utf8").toString("base64");

const TIMEOUT_WAIT_MS = 200; // > terminal.ts:13 的 150 ms
const SHORT_WAIT_MS = 30; // 介于 StdinBuffer 的 escapeTimeout(10 ms) 与协商 150 ms 之间

async function runCase(id, steps) {
	const events = [];
	const fakeOut = { write: () => true, isTTY: false, columns: 100, rows: 30, on: () => {}, removeListener: () => {} };
	const fakeIn = Object.assign(new EventEmitter(), {
		isTTY: true, isRaw: false, setRawMode() { return this; }, setEncoding() { return this; }, resume() { return this; }, pause() { return this; },
	});
	const origOut = process.stdout, origIn = process.stdin;
	Object.defineProperty(process, "stdout", { value: fakeOut, configurable: true });
	Object.defineProperty(process, "stdin", { value: fakeIn, configurable: true });
	const perStep = [];
	try {
		const term = new ProcessTerminal();
		term.start((data) => events.push(data), () => {});
		await sleep(30); // 让启动序列写出、协商缓冲就绪
		events.length = 0;
		for (const step of steps) {
			if (step.feed) {
				for (const chunk of step.feed) {
					fakeIn.emit("data", chunk);
					await sleep(3); // 仅让 StdinBuffer 完成切分（远小于 150 ms）
				}
			}
			if (step.shortWait) await sleep(SHORT_WAIT_MS);
			if (step.wait) await sleep(TIMEOUT_WAIT_MS);
			perStep.push({ name: step.name ?? "step", events: events.splice(0).map(b64) });
		}
		term.stop();
	} finally {
		Object.defineProperty(process, "stdout", { value: origOut, configurable: true });
		Object.defineProperty(process, "stdin", { value: origIn, configurable: true });
	}
	return { id, steps: steps.map((s) => ({ name: s.name ?? "step", feed: (s.feed ?? []).map(b64), wait: Boolean(s.wait), shortWait: Boolean(s.shortWait) })), perStep };
}

const cases = [];
const add = async (id, steps) => cases.push(await runCase(id, steps));

// ① 孤立 ESC：应**立即**转发（不进协商缓冲）
await add("t1-lone-esc-immediate", [{ feed: ["\x1b"], name: "esc" }]);
await add("t2-lone-esc-then-key", [{ feed: ["\x1b"], name: "esc" }, { feed: ["a"], name: "key" }]);
await add("t3-lone-esc-then-wait", [{ feed: ["\x1b"], name: "esc" }, { wait: true, name: "after-wait" }]);
// ② 协商前缀：应挂起，超时后才转发
await add("t4-prefix-esc-bracket", [{ feed: ["\x1b["], name: "prefix" }, { wait: true, name: "after-wait" }]);
await add("t5-prefix-esc-?-empty", [{ feed: ["\x1b[?"], name: "prefix" }, { wait: true, name: "after-wait" }]);
await add("t6-prefix-esc-?-digits", [{ feed: ["\x1b[?12"], name: "prefix" }, { wait: true, name: "after-wait" }]);
// ③ 非协商 CSI：应立即转发（不进协商缓冲）
await add("t7-csi-arrow-mod", [{ feed: ["\x1b[1;5A"], name: "csi" }]);
await add("t8-csi-delete", [{ feed: ["\x1b[3~"], name: "csi" }]);
await add("t9-csi-pageup", [{ feed: ["\x1b[5~"], name: "csi" }]);
// ②b 短等待（30 ms，介于 StdinBuffer 10 ms 与协商 150 ms 之间）：孤立 ESC 应已转发，协商前缀应仍挂起
await add("t13-lone-esc-short-wait", [{ feed: ["\x1b"], name: "esc" }, { shortWait: true, name: "after-30ms" }]);
await add("t14-prefix-short-wait", [{ feed: ["\x1b[?"], name: "prefix" }, { shortWait: true, name: "after-30ms" }]);
await add("t15-csi-short-wait", [{ feed: ["\x1b[1;5A"], name: "csi" }, { shortWait: true, name: "after-30ms" }]);
// ④ 完整应答：被消费（无事件）
await add("t10-kitty-answer", [{ feed: ["\x1b[?1u"], name: "answer" }]);
await add("t11-da1-answer", [{ feed: ["\x1b[?1;2c"], name: "answer" }]);
await add("t12-prefix-then-complete", [{ feed: ["\x1b[?"], name: "prefix" }, { feed: ["1u"], name: "complete" }]);

const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-term-timing.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-term-timing.mjs", tsVersion: "0.87.1", timeoutWaitMs: TIMEOUT_WAIT_MS, cases }, null, 0));
console.error(`转发时机夹具：用例=${cases.length} 大小=${(fs.statSync(out).size / 1024).toFixed(1)} KB`);
