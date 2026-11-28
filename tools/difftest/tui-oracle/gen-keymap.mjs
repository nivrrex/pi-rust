// P7-6af/A.3：**键名 oracle**——用 ref 真实 `parseKey`（`keys.ts:1251`）产出期望键名，**不由 Rust 反推**。
import fs from "node:fs";
import path from "node:path";
const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const keys = await import(path.join(REF, "keys.ts"));
const b64 = (s) => Buffer.from(String(s), "utf8").toString("base64");

const SEQS = [
	["cr", "\r"], ["lf", "\n"], ["tab", "\t"], ["del-7f", "\x7f"], ["bs-08", "\x08"],
	["shift-tab-csiz", "\x1b[Z"], ["delete", "\x1b[3~"], ["pageup", "\x1b[5~"], ["pagedown", "\x1b[6~"],
	["ctrl-a", "\x01"], ["ctrl-b", "\x02"], ["ctrl-c", "\x03"], ["ctrl-d", "\x04"], ["ctrl-e", "\x05"],
	["ctrl-f", "\x06"], ["ctrl-g", "\x07"], ["ctrl-i", "\x09"], ["ctrl-j", "\x0a"], ["ctrl-k", "\x0b"],
	["ctrl-l", "\x0c"], ["ctrl-m", "\x0d"], ["ctrl-n", "\x0e"], ["ctrl-o", "\x0f"], ["ctrl-p", "\x10"],
	["ctrl-q", "\x11"], ["ctrl-r", "\x12"], ["ctrl-s", "\x13"], ["ctrl-t", "\x14"], ["ctrl-u", "\x15"],
	["ctrl-v", "\x16"], ["ctrl-w", "\x17"], ["ctrl-x", "\x18"], ["ctrl-y", "\x19"], ["ctrl-z", "\x1a"],
	["alt-a", "\x1ba"], ["alt-z", "\x1bz"], ["space", " "], ["esc", "\x1b"],
	["home", "\x1b[H"], ["end", "\x1b[F"], ["insert", "\x1b[2~"], ["f1", "\x1bOP"],
];

const cases = [];
for (const kitty of [false, true]) {
	keys.setKittyProtocolActive(kitty);
	for (const [id, seq] of SEQS) {
		cases.push({ id: `${id}${kitty ? "-kitty" : ""}`, seq: b64(seq), kitty, key: keys.parseKey(seq) ?? null });
	}
}
const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-keymap.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-keymap.mjs", tsVersion: "0.87.1", cases }, null, 0));
const lines = [];
for (const c of cases) lines.push(`case ${c.id} ${c.kitty ? 1 : 0} ${c.seq} ${c.key ?? "-"}`);
const side = out.replace(/\.json$/, ".lines.txt");
fs.writeFileSync(side, lines.join("\n") + "\n");
console.error(`键名夹具：用例=${cases.length} 大小=${(fs.statSync(out).size / 1024).toFixed(1)} KB`);
