// P7-6k/W1：oracle 夹具生成器。
// 驱动 ref/pi-0.87.1/packages/tui 的**真实** `TuiMainScreen`，用假终端捕获每次 doRender 写出的字节。
// 不复制上游源码：运行时从 REF_DIR（默认 ref 只读目录）直接 import .ts（node ≥22.18 类型剥离）。
// 依赖：`get-east-asian-width`、`marked` 由 NODE_PATH 指向已安装包的 node_modules（不新增 npm 依赖）。
import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const REF_DIR = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { TuiMainScreen } = await import(path.join(REF_DIR, "tui-main-screen.ts"));
const { Container } = await import(path.join(REF_DIR, "tui.ts"));
const { visibleWidth, truncateToWidth } = await import(path.join(REF_DIR, "utils.ts"));

/** 固定种子 PRNG（mulberry32），脚本内自实现，无外部依赖。 */
function mulberry32(seed) {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) >>> 0;
		let t = a;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

class FakeTerminal {
	constructor(cols, rows) {
		this._cols = cols;
		this._rows = rows;
		this.chunks = [];
		this.resizes = 0;
	}
	start() {}
	stop() {}
	async drainInput() {}
	write(data) {
		this.chunks.push(data);
	}
	get columns() {
		return this._cols;
	}
	get rows() {
		return this._rows;
	}
	get kittyProtocolActive() {
		return false;
	}
	// 以下操作不计入 oracle 字节（与 diff_render 的字节流无关），仅记录调用次数供诊断。
	moveBy() {}
	hideCursor() {
		this.cursorOps = (this.cursorOps ?? 0) + 1;
	}
	showCursor() {}
	clearLine() {}
	clearFromCursor() {}
	clearScreen() {}
	setTitle() {}
	setProgress() {}
	reset() {
		this.chunks = [];
	}
	take() {
		const s = this.chunks.join("");
		this.chunks = [];
		return s;
	}
}

/** 被渲染的“行数组”组件：render(width) 直接返回夹具给定的行（按宽度截断/补齐由夹具保证）。 */
class LinesComponent extends Container {
	constructor() {
		super();
		this.lines = [];
	}
	setLines(lines) {
		this.lines = lines;
	}
	render(width) {
		// 上游要求组件自行截断（否则 doRender 抛错）：用上游的 visibleWidth/truncateToWidth 保证 CJK 宽度正确。
		return this.lines.map((line) => (visibleWidth(line) > width ? truncateToWidth(line, width) : line));
	}
}

function runCase(id, seed, frames, sizes) {
	const term = new FakeTerminal(sizes[0].width, sizes[0].height);
	const tui = new TuiMainScreen(term);
	const comp = new LinesComponent();
	tui.addChild(comp);
	const outputs = [];
	const recorded = [];
	frames.forEach((lines, index) => {
		const { width, height } = sizes[index];
		term._cols = width;
		term._rows = height;
		comp.setLines(lines);
		term.reset();
		// doRender 是 protected（TS），运行时为普通方法 ⇒ 直接调用以避免 requestRender 的 16ms 节流。
		tui.doRender();
		const bytes = term.take();
		recorded.push(bytes);
		outputs.push(Buffer.from(bytes, "utf8").toString("base64"));
	});
	return { id, seed, frames: sizes.map((s, i) => ({ width: s.width, height: s.height, lines: frames[i] })), outputs };
}

// ---------- 用例集 ----------
const pad = (text, width) => {
	const padLen = Math.max(0, width - visibleWidth(text));
	return `${text}${" ".repeat(padLen)}`;
};
const filler = (n, width, prefix = "line") => Array.from({ length: n }, (_, i) => pad(`${prefix} ${i + 1}`, width));

const cases = [];

// (i) 手写用例（10 条）
{
	const w = 40;
	const h = 10;
	// 1 首次渲染
	cases.push(runCase("h01-first-render", 1, [filler(3, w)], [{ width: w, height: h }]));
	// 2 内容不变
	cases.push(runCase("h02-unchanged", 2, [filler(3, w), filler(3, w)], [{ width: w, height: h }, { width: w, height: h }]));
	// 3 末尾追加
	cases.push(runCase("h03-append-tail", 3, [filler(3, w), filler(5, w)], [{ width: w, height: h }, { width: w, height: h }]));
	// 4 改末行
	cases.push(
		runCase("h04-change-last", 4, [filler(4, w), [...filler(3, w), pad("CHANGED LAST", w)]], [
			{ width: w, height: h },
			{ width: w, height: h },
		]),
	);
	// 5 改中间行
	cases.push(
		runCase(
			"h05-change-middle",
			5,
			[
				filler(6, w),
				[...filler(2, w), pad("MIDDLE!", w), ...filler(3, w, "l")].slice(0, 6),
			],
			[{ width: w, height: h }, { width: w, height: h }],
		),
	);
	// 6 改视口之上的行（20 行内容、视口 10 行 ⇒ 第 0 行在视口之上）
	const tall = filler(20, w);
	const tallChanged = [pad("TOP CHANGED", w), ...tall.slice(1)];
	cases.push(runCase("h06-change-above", 6, [tall, tallChanged], [{ width: w, height: h }, { width: w, height: h }]));
	// 7 内容缩短
	cases.push(runCase("h07-shrink", 7, [filler(8, w), filler(4, w)], [{ width: w, height: h }, { width: w, height: h }]));
	// 8 从空增长到超过 rows
	cases.push(
		runCase("h08-grow-past-rows", 8, [[], filler(3, w), filler(25, w)], [
			{ width: w, height: h },
			{ width: w, height: h },
			{ width: w, height: h },
		]),
	);
	// 9 宽度变化
	cases.push(
		runCase("h09-width-change", 9, [filler(5, 20), filler(5, 30)], [
			{ width: 20, height: h },
			{ width: 30, height: h },
		]),
	);
	// 10 高度变化
	cases.push(
		runCase("h10-height-change", 10, [filler(5, w), filler(5, w)], [
			{ width: w, height: 8 },
			{ width: w, height: 14 },
		]),
	);
}

// (ii) 随机序列（200 条）
const ascii = ["alpha", "beta", "gamma", "delta", "hello world", "x", "TODO item", "0123456789"];
const cjk = ["中文内容", "全角字符ＡＢＣ", "日本語テキスト", "한국어", "混合 mixed 中文"];
const sgr = ["\x1b[31mred\x1b[0m", "\x1b[1mbold\x1b[0m", "\x1b[32mgreen\x1b[0m text"];
for (let n = 0; n < 200; n += 1) {
	const seed = 1000 + n;
	const rand = mulberry32(seed);
	const width = 20 + Math.floor(rand() * 61);
	const height = 5 + Math.floor(rand() * 26);
	const frameCount = 2 + Math.floor(rand() * 11);
	let lines = [];
	const frames = [];
	const sizes = [];
	for (let f = 0; f < frameCount; f += 1) {
		const ops = 1 + Math.floor(rand() * 4);
		for (let k = 0; k < ops; k += 1) {
			const roll = rand();
			if (roll < 0.35 || lines.length === 0) {
				const kind = rand();
				const text = kind < 0.45 ? ascii[Math.floor(rand() * ascii.length)] : kind < 0.8 ? cjk[Math.floor(rand() * cjk.length)] : sgr[Math.floor(rand() * sgr.length)];
				lines = [...lines, pad(`${text} ${Math.floor(rand() * 100)}`, width)];
			} else if (roll < 0.7) {
				const idx = Math.floor(rand() * lines.length);
				const text = rand() < 0.5 ? ascii[Math.floor(rand() * ascii.length)] : cjk[Math.floor(rand() * cjk.length)];
				lines = lines.map((line, i) => (i === idx ? pad(text, width) : line));
			} else if (roll < 0.85) {
				lines = lines.filter((_, i) => i !== Math.floor(rand() * lines.length));
			} else {
				// 插入空行
				const idx = Math.floor(rand() * (lines.length + 1));
				lines = [...lines.slice(0, idx), pad("", width), ...lines.slice(idx)];
			}
		}
		lines = lines.map((line) => (line.length > width ? line.slice(0, width) : line));
		frames.push(lines);
		sizes.push({ width, height });
	}
	cases.push(runCase(`r${String(n).padStart(3, "0")}`, seed, frames, sizes));
}

const outPath = process.argv[2] ?? new URL("./fixtures.json", import.meta.url).pathname;
const payload = { generator: "tools/difftest/tui-oracle/gen.mjs", tsVersion: "0.87.1", cases };
fs.writeFileSync(outPath, JSON.stringify(payload));
// 供 Rust 单测读取的**无依赖文本侧车**（同一份数据；JSON 仍为正式夹具）：
//   case <id> <seed> <帧数>
//   frame <width> <height> <行数>
//   line <base64(utf8)>
//   out  <base64(utf8)>
const sidecarPath = outPath.replace(/\.json$/, ".lines.txt");
const b64 = (s) => Buffer.from(s, "utf8").toString("base64");
const sidecar = [];
for (const c of cases) {
	sidecar.push(`case ${c.id} ${c.seed} ${c.frames.length}`);
	c.frames.forEach((f, i) => {
		sidecar.push(`frame ${f.width} ${f.height} ${f.lines.length}`);
		for (const line of f.lines) sidecar.push(`line ${b64(line)}`);
		sidecar.push(`out ${c.outputs[i]}`);
	});
}
fs.writeFileSync(sidecarPath, `${sidecar.join("\n")}\n`);
const bytes = fs.statSync(outPath).size;
console.log(`用例数=${cases.length}（手写 10 + 随机 200） 输出=${outPath} 大小=${(bytes / 1024).toFixed(1)} KB 侧车=${sidecarPath} ${(fs.statSync(sidecarPath).size / 1024).toFixed(1)} KB`);
const first = cases[0];
console.log(`样例 ${first.id}: 帧数=${first.frames.length} 帧0 输出字节=${Buffer.from(first.outputs[0], "base64").length}`);
