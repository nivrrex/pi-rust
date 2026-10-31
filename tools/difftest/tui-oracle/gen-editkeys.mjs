// P7-6am/K：**键位 oracle**——用 ref 真实 `Editor` 产出「初始文本 + 光标 + 按键 → 编辑后文本/光标」。
//
// 不复制上游源码；ref 的 `Editor` 与 `keybindings` 直接 import，`core/keybindings` 用已安装包的 dist。
import fs from "node:fs";
import path from "node:path";

const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const DIST = process.env.PI_TS_DIST ?? (() => { throw new Error("请设置 PI_TS_DIST=<pi-ts 的 dist 目录>"); })();
const { Editor } = await import(path.join(REF, "components/editor.ts"));
const kbMod = await import(path.join(REF, "keybindings.ts"));
const core = await import(path.join(DIST, "core/keybindings.js"));

const defs = [
	...Object.entries(kbMod.TUI_KEYBINDINGS).map(([id, v]) => ({ id, ...v })),
	...Object.entries(core.KEYBINDINGS).map(([id, v]) => ({ id, ...v })),
];
kbMod.setKeybindings(new core.KeybindingsManager(defs));

const ident = (s) => s;
const theme = new Proxy({}, { get: () => ident });
const tui = { requestRender() {}, requestImmediateRender() {}, onResize() {}, setFocus() {} };

/** 键名 → 送入编辑器的字节序列（**legacy 形态**，ref 的 Editor 自己解析）。 */
const KEYS = {
	"ctrl+d": "\x04",
	"ctrl+w": "\x17",
	"alt+backspace": "\x1b\x7f",
	"ctrl+j": "\n",
	"shift+enter": "\x1b[13;2u",
	"alt+enter": "\x1b\r",
	"left": "\x1b[D",
};

const CASES = [];
function add(id, text, moves, key) {
	const ed = new Editor(tui, theme, { paddingX: 1 });
	ed.setText(text);
	for (let i = 0; i < moves; i += 1) ed.handleInput(KEYS.left);
	const beforeText = ed.getText();
	ed.handleInput(KEYS[key]);
	const afterText = ed.getText();
	CASES.push({
		id,
		text: beforeText,
		moves,
		key,
		seq: Buffer.from(KEYS[key], "utf8").toString("base64"),
		expect_text: afterText,
	});
}

for (const key of ["ctrl+d", "ctrl+w", "alt+backspace", "ctrl+j", "shift+enter", "alt+enter"]) {
	add(`${key}-end`, "hello world", 0, key);
	add(`${key}-middle`, "hello world", 5, key);
	add(`${key}-empty`, "", 0, key);
	add(`${key}-single`, "a", 0, key);
	add(`${key}-multiline`, "line one\nline two", 0, key);
}

const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-editkeys.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-editkeys.mjs", tsVersion: "0.87.1", cases: CASES }, null, 0));
const lines = CASES.map((c) => [
	`case ${c.id}`,
	`text ${Buffer.from(c.text, "utf8").toString("base64")}`,
	`moves ${c.moves}`,
	`seq ${c.seq}`,
	`expect_text ${Buffer.from(c.expect_text, "utf8").toString("base64")}`,
].join("\n"));
fs.writeFileSync(out.replace(/\.json$/, ".lines.txt"), lines.join("\n") + "\n");
console.error(`键位夹具：用例=${CASES.length}`);
