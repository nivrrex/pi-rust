// P7-6s/W4：**终端协商 oracle**（假 stdin/stdout，实例化 ref 真实 `ProcessTerminal`）。
//
// 不依赖真实 TTY：用 EventEmitter 假 stdin（可 `emit("data", …)` 注入字节）+ 捕获式假 stdout。
// 时间：ref 的"分片聚合"用 150ms 定时器（`terminal.ts:13`）；本脚本对相关用例**显式等待 > 150ms**
// 让定时器必然到期，保证"超时后把残留当普通输入"这一分支可确定性复现（不引入随机时序）。
//
// 每例记录：分阶段写出字节、kitty 激活时序、交给输入处理器的事件序列、残留未消费字节。
import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";

const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { ProcessTerminal } = await import(path.join(REF, "terminal.ts"));
const keys = await import(path.join(REF, "keys.ts"));

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function runCase(id, steps) {
	const written = [];
	const events = [];
	const kittyTimeline = [];
	const fakeOut = {
		write: (s) => { written.push(String(s)); return true; },
		isTTY: false, columns: 100, rows: 30,
		on: () => {}, removeListener: () => {},
	};
	const fakeIn = Object.assign(new EventEmitter(), {
		isTTY: true, isRaw: false,
		setRawMode() { return this; }, setEncoding() { return this; },
		resume() { return this; }, pause() { return this; },
	});
	const origOut = process.stdout;
	const origIn = process.stdin;
	Object.defineProperty(process, "stdout", { value: fakeOut, configurable: true });
	Object.defineProperty(process, "stdin", { value: fakeIn, configurable: true });
	const phases = [];
	const mark = (name) => phases.push({ name, bytes: written.splice(0).join("") });
	try {
		const term = new ProcessTerminal();
		term.start((data) => { events.push(data); kittyTimeline.push({ at: `event:${data.length}`, kitty: keys.isKittyProtocolActive() }); }, () => {});
		mark("start");
		for (const step of steps) {
			if (step.feed !== undefined) {
				for (const chunk of step.feed) {
					fakeIn.emit("data", chunk);
					await sleep(step.waitAfterChunkMs ?? 5);
				}
			}
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
	return {
		id,
		phases,
		kittyTimeline,
		events,
		// "残留未消费字节"：ref 把未消费字节作为 data 事件交出；此处以事件序列表达
		leftover: events.join(""),
	};
}

const cases = [];
const add = async (id, steps) => cases.push(await runCase(id, steps));

// ① 无任何应答 ⇒ 降级路径（不会收到 kitty 应答；普通键照常转发）
await add("n1-no-response-key", [{ feed: ["a"], name: "key" }]);
await add("n2-no-response-arrow", [{ feed: ["\x1b[A"], name: "arrow" }]);
await add("n3-no-response-paste", [{ feed: ["\x1b[200~hi\x1b[201~"], name: "paste" }]);
// ② kitty 应答（flags=1 激活；flags=0 降级到 modifyOtherKeys）
await add("k1-kitty-flags1", [{ feed: ["\x1b[?1u"], waitMs: 200, name: "kitty" }]);
await add("k2-kitty-flags0", [{ feed: ["\x1b[?0u"], waitMs: 200, name: "flags0" }]);
await add("k3-kitty-flags5-then-key", [{ feed: ["\x1b[?5u"], waitMs: 200, name: "kitty5" }, { feed: ["\x1b[97;5u"], name: "key" }]);
// ③ DA1 应答
await add("d1-da1-only", [{ feed: ["\x1b[?1;2c"], waitMs: 200, name: "da1" }]);
await add("d2-da1-then-key", [{ feed: ["\x1b[?1;2c"], waitMs: 200, name: "da1" }, { feed: ["x"], name: "key" }]);
await add("d3-da1-after-kitty", [{ feed: ["\x1b[?1u"], waitMs: 200, name: "kitty" }, { feed: ["\x1b[?1;2c"], waitMs: 200, name: "da1" }]);
// ④ OSC 11 应答
await add("o1-osc11-bel", [{ feed: ["\x1b]11;rgb:0000/0000/0000\x07"], waitMs: 200, name: "osc11" }]);
await add("o2-osc11-st", [{ feed: ["\x1b]11;rgb:ffff/ffff/ffff\x1b\\"], waitMs: 200, name: "osc11st" }]);
await add("o3-osc11-then-key", [{ feed: ["\x1b]11;rgb:1111/2222/3333\x07"], waitMs: 200, name: "osc11" }, { feed: ["k"], name: "key" }]);
// ⑤ 应答被分片切开
await add("f1-kitty-split", [{ feed: ["\x1b[?", "1u"], waitMs: 200, name: "split" }]);
await add("f2-da1-split", [{ feed: ["\x1b[?1", ";2c"], waitMs: 200, name: "split" }]);
await add("f3-kitty-split-3", [{ feed: ["\x1b", "[?1", "u"], waitMs: 200, name: "split3" }]);
// ⑥ 应答在超时之后到达（先喂半个应答，等 >150ms，再喂后半）
await add("t1-late-tail", [{ feed: ["\x1b[?"], waitMs: 250, name: "half" }, { feed: ["1u"], waitMs: 200, name: "tail" }]);
await add("t2-late-da1", [{ feed: ["\x1b[?1"], waitMs: 250, name: "half" }, { feed: [";2c"], waitMs: 200, name: "tail" }]);
await add("t3-late-after-key", [{ feed: ["\x1b[?"], waitMs: 250, name: "half" }, { feed: ["z"], name: "key" }, { feed: ["1u"], waitMs: 200, name: "tail" }]);
// ⑦ 应答与普通按键字节交织
await add("i1-key-then-kitty", [{ feed: ["a\x1b[?1u"], waitMs: 200, name: "mixed" }]);
await add("i2-kitty-then-key", [{ feed: ["\x1b[?1ub"], waitMs: 200, name: "mixed" }]);
await add("i3-key-kitty-key", [{ feed: ["c"], name: "k1" }, { feed: ["\x1b[?1u"], waitMs: 200, name: "kitty" }, { feed: ["d"], name: "k2" }]);
// ⑧ stop 恢复序列
await add("s1-stop-after-kitty", [{ feed: ["\x1b[?1u"], waitMs: 200, name: "kitty" }]);
await add("s2-stop-after-flags0", [{ feed: ["\x1b[?0u"], waitMs: 200, name: "flags0" }]);
await add("s3-stop-plain", []);

const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-term.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-term.mjs", tsVersion: "0.87.1", cases }, null, 0));
const stat = fs.statSync(out);
console.error(`终端协商夹具：用例=${cases.length} 大小=${(stat.size / 1024).toFixed(1)} KB 输出=${out}`);
