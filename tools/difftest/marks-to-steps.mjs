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
// `--per-mark-dims`：每个 mark 带自己的 `cols`/`rows`（F54/Z3 的缩放场景，
// 缩放前后屏幕宽度不同）。缺省沿用 `meta.cols`/`meta.rows`（既有调用方行为不变）。
const perMarkDims = argv.includes("--per-mark-dims");
const cols = meta.cols;
const rows = meta.rows;
mkdirSync(outDir, { recursive: true });

const marks = meta.snapshotOffsets ?? meta.marks ?? [];
marks.forEach((mark, index) => {
  // 每个 mark 独立截断：与 `compare-tui.mjs` 的 golden 口径相同（只比最后一个快照时，
  // 它会截到最后一个 offset；这里逐个还原，等价于"那一步结束时屏幕长这样"）。
  const markCols = perMarkDims && mark.cols ? mark.cols : cols;
  const markRows = perMarkDims && mark.rows ? mark.rows : rows;
  // `--frame-backtrack`：把偏移**回退到它之前最后一个同步帧结束**（`\u001b[?2026l`）。
  // 理由：两侧的"已写字节数"各自记账，可能落在某个帧的中间（实测 pi-rs 侧一次
  // `• high` 被切在 `hig` 处）⇒ 截断点必须落在帧边界，否则还原出的屏幕缺最后 1 列。
  let offset = mark.offset;
  if (argv.includes("--frame-backtrack")) {
    // 把偏移**吸附到帧边界**：取"结束位置 >= offset 的**第一个** `?2026l`"。
    //
    // 为什么不是"回退到上一个帧尾"：pi-rs 钩子记的是 `payload.len()`（逻辑字节数），
    // 而 PTY 的 `ONLCR` 会把每个 `\n` 写成 `\r\n` ⇒ 实际流里每帧都比记录值长几个字节，
    // 偏移永远落在**当前帧内部**。向前吸附才包含完整帧；向后吸附会退回上一帧、
    // 丢掉这一步的内容（实测：`[Extensions]` 展开态整段消失）。
    const syncEnd = "\u001b[?2026l";
    const text = raw.toString("utf8");
    let cursor = 0;
    let snapped = -1;
    while (cursor < text.length) {
      const index = text.indexOf(syncEnd, cursor);
      if (index < 0) break;
      const end = Buffer.byteLength(text.slice(0, index + syncEnd.length), "utf8");
      if (end >= offset) {
        snapped = end;
        break;
      }
      cursor = index + syncEnd.length;
    }
    if (snapped >= 0) offset = snapped;
  }
  const screen = normalizeScreen(
    renderScreen(trimRawToOffset(raw, offset), markCols, markRows),
  );
  writeFileSync(path.join(outDir, `step-${index}.txt`), `${screen.join("\n")}\n`);
});
writeFileSync(path.join(outDir, "steps.json"), JSON.stringify({ cols, rows, steps: marks.length }));
console.log(`marks-to-steps: ${marks.length} 步 → ${outDir}`);
