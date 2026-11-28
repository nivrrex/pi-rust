import fs from "node:fs";
import path from "node:path";
import { normalizeScreen } from "./compare-tui.mjs";
import { dumpStyled } from "./color-dump.mjs";

// 仓库根按本文件位置推导（禁止硬编码本机绝对路径）。
const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../..");
const goldDir = path.join(repo, "golden-tui");
const runDir = process.env.CLASSIFY_RUN_DIR ?? "/tmp/regress-tui";
const lines = (path) => dumpStyled(path, 100, 30).map((l) => l.replace(/^\d+│/, ""));
let total = 0, exact = 0, offsetOnly = 0, real = 0;
const samples = [];
for (const name of fs.readdirSync(goldDir)) {
  const g = `${goldDir}/${name}/run-1/raw.bin`;
  const r = `${runDir}/${name}/run-1/run-1/raw.bin`;
  if (!fs.existsSync(g) || !fs.existsSync(r)) continue;
  total += 1;
  const a = lines(g), b = lines(r);
  if (a.join("\n") === b.join("\n")) { exact += 1; continue; }
  const na = normalizeScreen(a).filter((l) => !/^│-$/.test(l));
  const nb = normalizeScreen(b).filter((l) => !/^│-$/.test(l));
  if (na.join("\n") === nb.join("\n")) { exact += 1; continue; }
  const sa = na.filter((l) => !/^│-$/.test(l) && l.replace(/│.*$/, "") !== "");
  const blankA = na.filter((l) => /^│-$/.test(l)).length, blankB = nb.filter((l) => /^│-$/.test(l)).length;
  if (na.length !== nb.length && na.filter(Boolean).join("\n") === nb.filter(Boolean).join("\n")) { offsetOnly += 1; continue; }
  real += 1;
  if (samples.length < 6) samples.push(name);
  void sa; void blankA; void blankB;
}
console.log(`total=${total} exact=${exact} offset=${offsetOnly} real=${real}`);
console.log("real:", samples.join(" "));
