// 打印某次 run 的**快照帧**（按 meta.json 的 snapshotOffsets），每行带列号，
// 用于 overlay / 面板类场景的几何取证（compare-tui 只比末帧，看不到中间帧）。
//
// 用法：node tools/difftest/dump-snapshot.mjs <run 目录（含 raw.bin/meta.json）> [--cols N] [--rows N]
import fs from "node:fs";
import path from "node:path";
import { renderScreen } from "./compare-tui.mjs";

const dir = process.argv[2];
if (!dir) {
  console.error("用法：node dump-snapshot.mjs <run 目录>");
  process.exit(2);
}
const argOf = (name, fallback) => {
  const i = process.argv.indexOf(name);
  return i >= 0 ? Number(process.argv[i + 1]) : fallback;
};
const meta = JSON.parse(fs.readFileSync(path.join(dir, "meta.json"), "utf8"));
const cols = argOf("--cols", meta.cols ?? 100);
const rows = argOf("--rows", meta.rows ?? 30);
const raw = fs.readFileSync(path.join(dir, "raw.bin"));
// 用终止偏移把整段流切成"到该快照为止"的字节，再整段喂给终端模拟器。
const offsets = (meta.snapshotOffsets ?? []).map((item) => item.offset ?? item);
for (const [index, offset] of offsets.entries()) {
  const slice = raw.subarray(0, Number(offset));
  const screen = renderScreen(slice, cols, rows);
  console.log(`=== snapshot #${index} @${offset} ===`);
  screen.forEach((line, row) => {
    if (!line.trim()) return;
    console.log(`${String(row).padStart(2)}|${line}`);
  });
}
