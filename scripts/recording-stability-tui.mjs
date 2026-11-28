// TUI 录制稳定性（P7-2 W2）：run-2/run-3 对 run-1 快照必须全 same（用 compare-tui 的归一化 + 闸门）。
import { compareTui, lastSnapshotOffset, snapOffsetToFrameEnd, trimRawToOffset } from "../tools/difftest/compare-tui.mjs";
import fs from "fs";
import path from "path";

const argv = process.argv.slice(2);
const get = (n, d) => { const i = argv.indexOf(n); return i >= 0 ? argv[i + 1] : d; };
const dir = get("--dir");
const cols = Number.parseInt(get("--cols", "100"), 10);
const rows = Number.parseInt(get("--rows", "30"), 10);
const required = argv.reduce((acc, item, index) => { if (item === "--require") acc.push(argv[index + 1]); return acc; }, []);
if (!dir) { console.error("用法: node scripts/recording-stability-tui.mjs --dir <dir> --cols 100 --rows 30 [--require 文本]"); process.exit(2); }
const base = path.join(dir, "run-1", "raw.bin");
if (!fs.existsSync(base)) { console.error(`缺少基线快照: ${base}`); process.exit(2); }
const runs = fs.readdirSync(dir).filter((n) => /^run-\d+$/.test(n)).sort((a, b) => Number(a.slice(4)) - Number(b.slice(4)));
let same = 0, checked = 0;
for (const run of runs) {
  if (run === "run-1") continue;
  const other = path.join(dir, run, "raw.bin");
  if (!fs.existsSync(other)) continue;
  checked += 1;
  const trim = (rawPath) => {
    const raw = fs.readFileSync(rawPath);
    if (argv.includes("--no-trim")) return raw;
    const offset = lastSnapshotOffset(path.join(path.dirname(rawPath), "meta.json"));
    if (offset === null) return raw;
    const snapped = snapOffsetToFrameEnd(raw, offset);
    if (snapped === null) {
      console.error(`[tui-stability] invalid：偏移之前没有 ?2026l 帧边界 ${rawPath} offset=${offset}`);
      process.exit(2);
    }
    return trimRawToOffset(raw, snapped);
  };
  const result = compareTui(trim(base), trim(other), { cols, rows, required });
  console.log(`[tui-stability] ${run}: ${result.status}${result.reason ? ` (${result.reason})` : ""}`);
  if (result.status === "same") same += 1;
}
console.log(`[tui-stability] 一致 ${same}/${checked}`);
process.exit(checked > 0 && same === checked ? 0 : 1);
