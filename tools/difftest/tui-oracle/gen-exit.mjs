// P7-6m/W2：**退出序列夹具**生成器（独立文件 `fixtures-exit.json`；v1 夹具与 sha256 不动）。
// 与 v1 的差别：① 行内注入 `CURSOR_MARKER`（`\x1b_pi:c\x07`，tui.ts:168）；
//              ② 假终端把 `hideCursor/showCursor` 记为真实字节 `\x1b[?25l` / `\x1b[?25h`
//                 （v1 忽略它们，因此 v1 不含光标分支的字节）；
//              ③ 覆盖 `showHardwareCursor` 的两种取值。
import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const REF_DIR = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { TuiMainScreen } = await import(path.join(REF_DIR, "tui-main-screen.ts"));
const { Container, CURSOR_MARKER } = await import(path.join(REF_DIR, "tui.ts"));
const { visibleWidth, truncateToWidth } = await import(path.join(REF_DIR, "utils.ts"));

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
	moveBy(lines) {
		if (lines === 0) return;
		this.chunks.push(lines > 0 ? `\x1b[${lines}B` : `\x1b[${-lines}A`);
	}
	hideCursor() {
		this.chunks.push("\x1b[?25l");
	}
	showCursor() {
		this.chunks.push("\x1b[?25h");
	}
	clearLine() {
		this.chunks.push("\x1b[2K");
	}
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

class LinesComponent extends Container {
	constructor() {
		super();
		this.lines = [];
	}
	setLines(lines) {
		this.lines = lines;
	}
	render(width) {
		return this.lines.map((line) => (visibleWidth(line.replace(/\x1b_pi:c\x07/g, "")) > width ? truncateToWidth(line, width) : line));
	}
}

const pad = (text, width) => `${text}${" ".repeat(Math.max(0, width - visibleWidth(text)))}`;

function runCase(id, seed, frames, sizes, showHardwareCursor) {
	const term = new FakeTerminal(sizes[0].width, sizes[0].height);
	const tui = new TuiMainScreen(term, showHardwareCursor);
	const comp = new LinesComponent();
	tui.addChild(comp);
	const outputs = [];
	let stopBytes = "";
	frames.forEach((lines, index) => {
		term._cols = sizes[index].width;
		term._rows = sizes[index].height;
		comp.setLines(lines);
		term.reset();
		tui.doRender();
		outputs.push(Buffer.from(term.take(), "utf8").toString("base64"));
		if (index === frames.length - 1) {
			// P7-6p/W1：最后一帧后触发**停止路径**（`beforeTerminalStop`，`tui-main-screen.ts:168-176`），
			// 记录它单独写出的字节（`preserveScreen=false`，与真实退出一致）。
			term.reset();
			tui.beforeTerminalStop({ preserveScreen: false });
			stopBytes = Buffer.from(term.take(), "utf8").toString("base64");
		}
	});
	return {
		id,
		seed,
		cursorMarker: "\\u001b_pi:c\\u0007",
		showHardwareCursor,
		frames: sizes.map((s, i) => ({ width: s.width, height: s.height, lines: frames[i] })),
		outputs,
		stop: stopBytes,
	};
}

const cases = [];
const MARK = CURSOR_MARKER;

/** 在字符串的可见列 col 处插入 marker（跳过 ANSI 序列）。 */
function insertMarker(text, col) {
	let seen = 0;
	for (let i = 0; i < text.length; i += 1) {
		if (text[i] === "\x1b") {
			// 跳过 CSI/OSC/APC
			const next = text[i + 1];
			if (next === "[") {
				let j = i + 2;
				while (j < text.length && !/[A-Za-z]/.test(text[j])) j += 1;
				i = j;
				continue;
			}
			if (next === "]" || next === "_") {
				let j = i + 2;
				while (j < text.length && text[j] !== "\x07") j += 1;
				i = j;
				continue;
			}
		}
		if (seen === col) return text.slice(0, i) + MARK + text.slice(i);
		seen += 1;
	}
	return text + MARK;
}

// (i) 手写用例
const W = 40;
const H = 10;
const base = ["first line", "second line", "third line"];
const withMarker = (lines, row, col) => lines.map((l, i) => (i === row ? insertMarker(l, col) : l));
cases.push(runCase("c01-marker-start", 1, [withMarker(base, 1, 0)], [{ width: W, height: H }], false));
cases.push(runCase("c02-marker-middle", 2, [withMarker(base, 1, 3)], [{ width: W, height: H }], false));
cases.push(runCase("c03-marker-end", 3, [withMarker(base, 1, "second line".length)], [{ width: W, height: H }], false));
cases.push(runCase("c04-marker-after-cjk", 4, [withMarker(["中文内容ABC", "第二行"], 0, 4)], [{ width: W, height: H }], false));
cases.push(runCase("c05-marker-after-sgr", 5, [withMarker(["\x1b[31mred\x1b[0m text"], 0, 3)], [{ width: W, height: H }], false));
const tall = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`);
cases.push(runCase("c06-marker-in-viewport", 6, [withMarker(tall, 18, 2)], [{ width: W, height: H }], false));
cases.push(runCase("c07-marker-above-viewport", 7, [withMarker(tall, 1, 2)], [{ width: W, height: H }], false));
cases.push(runCase("c08-no-marker", 8, [base], [{ width: W, height: H }], false));
cases.push(runCase("c09-marker-line-removed", 9, [withMarker(base, 2, 1), base.slice(0, 2)], [{ width: W, height: H }, { width: W, height: H }], false));
cases.push(runCase("c10-marker-width-change", 10, [withMarker(base, 1, 2), withMarker(base, 1, 2)], [{ width: 20, height: H }, { width: 30, height: H }], false));
cases.push(runCase("c11-show-hw-cursor", 11, [withMarker(base, 1, 2)], [{ width: W, height: H }], true));
cases.push(runCase("c12-show-hw-no-marker", 12, [base], [{ width: W, height: H }], true));

// (ii) 随机 100 条（在 v1 随机基础上随机把 marker 插入某行）
const ascii = ["alpha", "beta", "gamma", "hello world", "TODO item"];
const cjk = ["中文内容", "全角字符ＡＢＣ", "日本語"];
const sgr = ["\x1b[31mred\x1b[0m", "\x1b[1mbold\x1b[0m"];
for (let n = 0; n < 100; n += 1) {
	const seed = 5000 + n;
	const rand = mulberry32(seed);
	const width = 20 + Math.floor(rand() * 61);
	const height = 5 + Math.floor(rand() * 26);
	const frameCount = 2 + Math.floor(rand() * 11);
	const showHardwareCursor = n % 2 === 1;
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
				lines = lines.map((l, i) => (i === idx ? pad("changed", width) : l));
			} else if (roll < 0.85) {
				lines = lines.filter((_, i) => i !== Math.floor(rand() * lines.length));
			} else {
				const idx = Math.floor(rand() * (lines.length + 1));
				lines = [...lines.slice(0, idx), pad("", width), ...lines.slice(idx)];
			}
		}
		// 每帧把 marker 随机插到某一行
		let stripped = lines.map((l) => l.replace(/\x1b_pi:c\x07/g, ""));
		if (stripped.length === 0) stripped = [pad("seed line", width)];
		const row = Math.floor(rand() * stripped.length);
		const col = Math.floor(rand() * Math.max(1, visibleWidth(stripped[row])));
		lines = stripped.map((l, i) => (i === row ? insertMarker(l, col) : l));
		frames.push(lines);
		sizes.push({ width, height });
	}
	cases.push(runCase(`cr${String(n).padStart(3, "0")}`, seed, frames, sizes, showHardwareCursor));
}

const outPath = process.argv[2] ?? new URL("./fixtures-exit.json", import.meta.url).pathname;
fs.writeFileSync(outPath, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-exit.mjs", tsVersion: "0.87.1", cursorMarker: "\\u001b_pi:c\\u0007", cases }));
// 供 Rust 单测读取的无依赖文本侧车（格式同 v1 `fixtures.lines.txt`，多一行 `show <0|1>`）。
const sidecarPath = outPath.replace(/\.json$/, ".lines.txt");
const b64 = (s) => Buffer.from(s, "utf8").toString("base64");
const sidecar = [];
for (const c of cases) {
	sidecar.push(`case ${c.id} ${c.seed} ${c.frames.length} ${c.showHardwareCursor ? 1 : 0}`);
	c.frames.forEach((f, i) => {
		sidecar.push(`frame ${f.width} ${f.height} ${f.lines.length}`);
		for (const line of f.lines) sidecar.push(`line ${b64(line)}`);
		sidecar.push(`out ${c.outputs[i]}`);
	});
}
fs.writeFileSync(sidecarPath, `${sidecar.join("\n")}\n`);
console.log(`退出序列夹具用例=${cases.length}（手写 12 + 随机 100） 输出=${outPath} 大小=${(fs.statSync(outPath).size / 1024).toFixed(1)} KB`);
