// P7-6an/K2：**ref 全量默认绑定表**导出（`TUI_KEYBINDINGS` 47 + `KEYBINDINGS` 90）。
import fs from "node:fs";
import path from "node:path";
const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const DIST = process.env.PI_TS_DIST ?? (() => { throw new Error("请设置 PI_TS_DIST=<pi-ts 的 dist 目录>"); })();
const tui = await import(path.join(REF, "keybindings.ts"));
const core = await import(path.join(DIST, "core/keybindings.js"));

const rows = [];
const norm = (keys) => (Array.isArray(keys) ? keys : [keys]).filter((k) => typeof k === "string" && k.length > 0);
for (const [id, def] of Object.entries(tui.TUI_KEYBINDINGS)) {
	rows.push({ id, source: "tui", keys: norm(def.defaultKeys), description: def.description ?? "" });
}
for (const [id, def] of Object.entries(core.KEYBINDINGS)) {
	rows.push({ id, source: "core", keys: norm(def.defaultKeys), description: def.description ?? "" });
}
const out = process.argv[2] ?? path.join(import.meta.dirname, "fixtures-keybindings.json");
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-keybindings.mjs", tsVersion: "0.87.1", rows }, null, 0));
fs.writeFileSync(out.replace(/\.json$/, ".lines.txt"), rows.map((r) => `${r.source} ${r.id} ${r.keys.join(",") || "-"}`).join("\n") + "\n");
console.error(`绑定表：tui=${rows.filter((r) => r.source === "tui").length} core=${rows.filter((r) => r.source === "core").length}`);
