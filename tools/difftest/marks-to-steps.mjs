#!/usr/bin/env node
// F53/T3：把 pty-helper 录制流按 mark 切成**逐步屏幕文本**。
//
// 与 `run-tui.mjs` 的 golden 截断口径一致：`meta.json` 的 `snapshotOffsets[i].offset`
// 是第 i 个 mark 时的字节偏移，截到该偏移后由 `compare-tui.mjs` 的 `renderScreen`
// 还原成 rows×cols 的屏幕；这里只把结果写成 `step-<i>.txt`（与 pi-rs 侧
// `PI_TEST_DUMP` 的 `step-N.txt` 同格式，供 `real-parity.sh` 逐行比较）。
//
// 用法：node tools/difftest/marks-to-steps.mjs --raw <raw.bin> --meta <meta.json> --out <dir>
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { renderScreen, normalizeScreen, trimRawToOffset } from "./compare-tui.mjs";

const argv = process.argv.slice(2);
const get = (name, fallback) => {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : fallback;
};

const rawPath = get("--raw");
const metaPath = get("--meta");
const outDir = get("--out");
if (!rawPath || !metaPath || !outDir) {
  console.error("用法: marks-to-steps.mjs --raw raw.bin --meta meta.json --out dir");
  process.exit(2);
}

const meta = JSON.parse(readFileSync(metaPath, "utf8"));
const raw = readFileSync(rawPath);
const cols = meta.cols;
const rows = meta.rows;
mkdirSync(outDir, { recursive: true });

const marks = meta.snapshotOffsets ?? [];
marks.forEach((mark, index) => {
  // 每个 mark 独立截断：与 `compare-tui.mjs` 的 golden 口径相同（只比最后一个快照时，
  // 它会截到最后一个 offset；这里逐个还原，等价于"那一步结束时屏幕长这样"）。
  const screen = normalizeScreen(renderScreen(trimRawToOffset(raw, mark.offset), cols, rows));
  writeFileSync(path.join(outDir, `step-${index}.txt`), `${screen.join("\n")}\n`);
});
writeFileSync(path.join(outDir, "steps.json"), JSON.stringify({ cols, rows, steps: marks.length }));
console.log(`marks-to-steps: ${marks.length} 步 → ${outDir}`);
