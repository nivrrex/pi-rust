// P7-6ag/M2：**kitty 激活态下的 `\n` 族键名** oracle（ref 真实 `parseKey`，不由 Rust 反推）。
import fs from "node:fs";
import path from "node:path";
const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const keys = await import(path.join(REF, "keys.ts"));
const b64 = (s) => Buffer.from(String(s), "utf8").toString("base64");

const SEQS = [
	["lf", "\n"], ["cr", "\r"], ["crlf", "\r\n"], ["esc-cr", "\x1b\r"], ["esc-lf", "\x1b\n"],
	["esc-om", "\x1bOM"], ["ctrl-j", "\x0a"], ["ctrl-m", "\x0d"], ["kitty-enter", "\x1b[13u"],
	["kitty-shift-enter", "\x1b[13;2u"], ["esc-space", "\x1b "], ["esc-bs", "\x1b\x7f"],
];
const cases = [];
for (const kitty of [false, true]) {
	keys.setKittyProtocolActive(kitty);
	for (const [id, seq] of SEQS) {
		cases.push({ id: `${id}${kitty ? "-kitty" : ""}`, seq: b64(seq), kitty, key: keys.parseKey(seq) ?? null });
	}
}
const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-keys-mode.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-keys-mode.mjs", tsVersion: "0.87.1", cases }, null, 0));
const lines = cases.map((c) => `case ${c.id} ${c.kitty ? 1 : 0} ${c.seq} ${c.key ?? "-"}`);
fs.writeFileSync(out.replace(/\.json$/, ".lines.txt"), lines.join("\n") + "\n");
console.error(`kitty 模式键名夹具：用例=${cases.length}`);
