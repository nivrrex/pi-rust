// P7-6an/C：**Tab 补全 oracle**——用 ref 真实 `CombinedAutocompleteProvider.applyCompletion`
// （`packages/tui/src/autocomplete.ts:389-418`）产出「输入 + 选中项 → 补全后文本/光标」。
import fs from "node:fs";
import path from "node:path";
const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { CombinedAutocompleteProvider } = await import(path.join(REF, "autocomplete.ts"));

const provider = new CombinedAutocompleteProvider([], process.cwd(), null);
const CASES = [
	["slash-settings", "/", "settings"],
	["s-settings", "/s", "settings"],
	["se-settings", "/se", "settings"],
	["set-settings", "/set", "settings"],
	["s-scoped", "/s", "scoped-models"],
	["scoped", "/sco", "scoped-models"],
	["m-model", "/m", "model"],
	["mo-model", "/mo", "model"],
	["q-quit", "/q", "quit"],
	["h-hotkeys", "/h", "hotkeys"],
	["e-export", "/e", "export"],
	["mid-cursor", "/se", "settings"],
	["with-suffix", "/se x", "settings"],
	["after-space", "/settings x", "settings"],
	["empty-line", "", "settings"],
];
const rows = CASES.map(([id, text, value]) => {
	const lines = [text];
	const cursorCol = text.length;
	const res = provider.applyCompletion(lines, 0, cursorCol, { value, label: value }, text);
	return { id, input: text, value, expect_text: res.lines[0] ?? "", expect_cursor: res.cursorCol };
});
const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-tabcomplete.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-tabcomplete.mjs", tsVersion: "0.87.1", rows }, null, 0));
fs.writeFileSync(out.replace(/\.json$/, ".lines.txt"), rows.map((r) =>
	`case ${r.id}\ninput ${Buffer.from(r.input, "utf8").toString("base64")}\nvalue ${r.value}\nexpect_text ${Buffer.from(r.expect_text, "utf8").toString("base64")}\nexpect_cursor ${r.expect_cursor}`).join("\n") + "\n");
console.error(`Tab 补全夹具：用例=${rows.length}`);
