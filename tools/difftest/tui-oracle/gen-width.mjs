// P7-6r0/W2：**面板宽度夹具**生成器（#15 方案 C）。
//
// 做法：从 ref 源码里**原样取出** `/hotkeys` 面板的 markdown 模板（`interactive-mode.ts` 的
// `let hotkeys = \`…\``），把模板里的 `${cursorUp}` 等变量用**真实的** `keyDisplayText()` 求值，
// 再交给**真实的** `Markdown` 组件（`@earendil-works/pi-tui`，与 ref `:6652` 同参 `(text,1,1,theme)`）
// 在 cols=60/80/100/120 渲染，输出未折叠行数组（含 ANSI 原样）。
//
// 不复制上游源码、不新增 npm 依赖、不改安装目录；模块路径由 `TS_PI_ENTRY` 推导。
import fs from "node:fs";
import path from "node:path";

const ENTRY = process.env.TS_PI_ENTRY;
if (!ENTRY) {
	console.error("需要 TS_PI_ENTRY 指向 pi-coding-agent 的 bundle/cli.js");
	process.exit(2);
}
const DIST = path.dirname(path.dirname(ENTRY)); // …/dist
const TUI = await import(path.join(DIST, "../node_modules/@earendil-works/pi-tui/dist/index.js"));
const { keyDisplayText } = await import(path.join(DIST, "modes/interactive/components/keybinding-hints.js"));
const { Markdown, stripTerminalSequences } = TUI;
// **真实调用链**：`startup-ui.js:59` 在启动时执行 `setKeybindings(KeybindingsManager.create())`；
// 不做这一步，`keyDisplayText("app.*")` 全部返回 ""（⇒ 行内代码为空、表格列宽被压到 5）。
const kbMod = await import(path.join(DIST, "../node_modules/@earendil-works/pi-tui/dist/keybindings.js"));
const kbCore = await import(path.join(DIST, "core/keybindings.js"));
// 与真实启动一致：tui 定义（`pi-tui` 的 TUI_KEYBINDINGS）+ app 定义（`core/keybindings.js` 的 KEYBINDINGS）
const definitions = [
	...Object.entries(kbMod.TUI_KEYBINDINGS).map(([id, v]) => ({ id, ...v })),
	...Object.entries(kbCore.KEYBINDINGS).map(([id, v]) => ({ id, ...v })),
];
kbMod.setKeybindings(new kbCore.KeybindingsManager(definitions));

const SRC = path.join(DIST, "../../src/modes/interactive/interactive-mode.ts");
const refPath = fs.existsSync(SRC) ? SRC : null;

/** 从 ref 源码取出 hotkeys 模板与变量绑定（不可用时回退到内置副本，见 README）。 */
function loadTemplate() {
	const candidates = [
		refPath,
		path.resolve(import.meta.dirname, "../../../ref/pi-0.87.1/packages/coding-agent/src/modes/interactive/interactive-mode.ts"),
	].filter(Boolean);
	for (const file of candidates) {
		if (!fs.existsSync(file)) continue;
		const text = fs.readFileSync(file, "utf8");
		const start = text.indexOf("let hotkeys = `");
		if (start < 0) continue;
		const open = text.indexOf("`", start);
		const close = text.indexOf("`;", open + 1);
		const template = text.slice(open + 1, close);
		// 变量绑定：`const X = this.getEditorKeyDisplay("...")` / `getAppKeyDisplay("...")`
		const bindings = {};
		for (const m of text.matchAll(/const (\w+) = this\.get(?:Editor|App)KeyDisplay\("([^"]+)"\)/g)) {
			bindings[m[1]] = keyDisplayText(m[2]);
		}
		return { file, template, bindings };
	}
	throw new Error("找不到 ref 的 hotkeys 模板");
}

const { file, template: rawTemplate, bindings } = loadTemplate();
// **关键**：从 TS 源码里取出的模板是**字面量源码**，其中的 `\`` 是转义写法；
// 必须先反转义（`\`` → `` ` ``、`\${` → `${`），否则 Markdown 会把 `\`` 当成
// "转义的反引号"而非**行内代码标记** ⇒ 行内代码不会被主题着色、反引号会原样出现。
const template = rawTemplate.replace(/\\`/g, "`").replace(/\\\$\{/g, "${");
const rendered = template.replace(/\$\{([^}]+)\}/g, (whole, expr) => {
	const key = expr.trim();
	if (key in bindings) return bindings[key];
	if (key === "process.platform") return process.platform;
	return whole; // 未绑定（如 process.platform 条件）原样保留
});
// `process.platform === "win32" ? … : ""` 这类表达式按非 win32 求值
const hotkeys = rendered.replace(/\$\{process\.platform === "win32" \? [^}]*\}/g, "").trim();

// **真实主题**：`getMarkdownTheme()`（ref `:6652` 用 `getMarkdownThemeWithSettings()`，
// 其核心即此函数 + 用户 settings 覆盖；此处取默认值，ANSI 原样保留）。
const themeModule = await import(path.join(DIST, "modes/interactive/theme/theme.js"));
themeModule.initTheme?.("dark");
const theme = themeModule.getMarkdownTheme();
const md = new Markdown(hotkeys, 1, 1, theme);

const widths = [60, 80, 100, 120];
const panels = {};
for (const width of widths) {
	const lines = md.render(width).map((l) => String(l));
	const plain = lines.map((l) => stripTerminalSequences(l));
	panels[String(width)] = {
		lines,
		plain,
		lineCount: lines.length,
		maxWidth: plain.reduce((m, l) => Math.max(m, l.length), 0),
	};
}

const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-width.json");
fs.writeFileSync(
	out,
	JSON.stringify({
		generator: "tools/difftest/tui-oracle/gen-width.mjs",
		tsVersion: "0.87.1",
		source: { file: path.basename(file), template: "interactive-mode.ts let hotkeys = `…`", render: "new Markdown(hotkeys.trim(), 1, 1, theme)" },
		widths,
		panels: { hotkeys: panels },
	}),
);
const stat = fs.statSync(out);
console.log(`宽度夹具：宽度=${widths.join("/")} 行数=${widths.map((w) => panels[String(w)].lineCount).join("/")} 最大宽=${widths.map((w) => panels[String(w)].maxWidth).join("/")} 大小=${(stat.size / 1024).toFixed(1)} KB`);
