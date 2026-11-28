// F35/G0c：把 md-oracle / md-render 的批量输出按**元素类型**聚合差异（只产出聚合表）。
//
// 用法：node tools/difftest/md-aggregate.mjs --dir tmp/g0 --picked tmp/g0/picked.json
import fs from "node:fs";
import path from "node:path";
import { diffLines } from "./md-cells.mjs";

const argv = process.argv.slice(2);
const get = (name, fallback) => {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : fallback;
};
const dir = path.resolve(get("--dir", "tmp/g0"));
const pickedPath = get("--picked", path.join(dir, "picked.json"));
const picked = JSON.parse(fs.readFileSync(pickedPath, "utf8"));
const kindOf = new Map();
for (const [kind, files] of Object.entries(picked)) {
  for (const file of files) if (!kindOf.has(file)) kindOf.set(file, kind);
}

const load = (name) => {
  const map = new Map();
  for (const line of fs.readFileSync(path.join(dir, name), "utf8").split("\n")) {
    if (!line.trim()) continue;
    const { file, lines } = JSON.parse(line);
    map.set(file, lines);
  }
  return map;
};

const byKind = {};
const rows = [];
for (const theme of ["dark", "light"]) {
  for (const width of [80, 120]) {
    const ts = load(`ts-${theme}-${width}.jsonl`);
    const rs = load(`rs-${theme}-${width}.jsonl`);
    for (const file of ts.keys()) {
      const a = ts.get(file);
      const b = rs.get(file) ?? [];
      const kind = kindOf.get(file) ?? "other";
      let cells = 0;
      let lines = 0;
      for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
        const { different } = diffLines(a[i] ?? "", b[i] ?? "");
        if (different > 0) {
          lines += 1;
          cells += different;
        }
      }
      const entry = (byKind[kind] ??= { checks: 0, diffChecks: 0, lines: 0, cells: 0 });
      entry.checks += 1;
      if (cells > 0) {
        entry.diffChecks += 1;
        entry.lines += lines;
        entry.cells += cells;
      }
      rows.push({ file, theme, width, kind, lines, cells });
    }
  }
}
const sorted = Object.entries(byKind).sort((x, y) => y[1].cells - x[1].cells);
console.log("kind            checks diff   lines      cells");
for (const [kind, v] of sorted) {
  console.log(
    kind.padEnd(14),
    String(v.checks).padStart(6),
    String(v.diffChecks).padStart(5),
    String(v.lines).padStart(7),
    String(v.cells).padStart(10),
  );
}
console.log("TOTAL cells:", rows.reduce((sum, row) => sum + row.cells, 0));
