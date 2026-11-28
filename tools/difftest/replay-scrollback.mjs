// P7-6j/W1：全文（scrollback + 可见窗口）重放（只读诊断，不入验收）。
//
// **实现策略（P7-6j 的教训）**：逐序列处理**与 `tools/difftest/compare-tui.mjs` 的 `renderScreen`
// 保持同一结构**（同样的 CSI 归属、同样的写入/夹取语义），**只**在"底行换行整屏上滚"那一步
// 把被移出屏幕的顶行 push 进 `scrollback`。这样"可见窗口"与 `renderScreen` 按构造一致。
// `2J` 清可见区；`3J` 清 scrollback（两者都计数并写进输出）。
import fs from "fs";
import { renderScreen, normalizeScreen, snapOffsetToFrameEnd } from "./compare-tui.mjs";

const CSI_FINAL = /[@-~]/;
const SYNC_END = Buffer.from("\u001b[?2026l");

export function replayFull(raw, cols, rows) {
  const text = raw.toString("utf8");
  const screen = Array.from({ length: rows }, () => Array.from({ length: cols }, () => " "));
  const scrollback = [];
  let r = 0;
  let c = 0;
  let i = 0;
  let clearsScreen = 0;
  let clearsScrollback = 0;
  const writeChar = (ch) => {
    const col = c >= cols ? cols - 1 : c;
    if (r < rows && col >= 0) screen[r][col] = ch;
    c += 1;
  };
  while (i < text.length) {
    const ch = text[i];
    if (ch === "\x1b") {
      const next = text[i + 1];
      if (next === "[") {
        let j = i + 2;
        while (j < text.length && !CSI_FINAL.test(text[j])) j += 1;
        const final = text[j] ?? "";
        const params = text.slice(i + 2, j);
        const nums = params.split(";").map((p) => (p === "" ? 0 : Number.parseInt(p, 10)));
        if (final === "H" || final === "f") {
          r = Math.min(Math.max((nums[0] || 1) - 1, 0), rows - 1);
          c = Math.min(Math.max((nums[1] || 1) - 1, 0), cols - 1);
        } else if (final === "A") r = Math.max(r - (nums[0] || 1), 0);
        else if (final === "B") r = Math.min(r + (nums[0] || 1), rows - 1);
        else if (final === "C") c = Math.min(c + (nums[0] || 1), cols - 1);
        else if (final === "D") c = Math.max(c - (nums[0] || 1), 0);
        else if (final === "J") {
          const mode = nums[0] || 0;
          if (mode === 2 || mode === 3) {
            if (mode === 3) { clearsScrollback += 1; scrollback.length = 0; }
            else { clearsScreen += 1; for (let y = 0; y < rows; y += 1) screen[y].fill(" "); }
          } else if (mode === 0) {
            for (let x = c; x < cols; x += 1) screen[r][x] = " ";
            for (let y = r + 1; y < rows; y += 1) screen[y].fill(" ");
          }
        } else if (final === "K") {
          const mode = nums[0] || 0;
          if (mode === 0) for (let x = c; x < cols; x += 1) screen[r][x] = " ";
          else if (mode === 2) screen[r].fill(" ");
        }
        i = j + 1;
        continue;
      }
      if (next === "]") {
        let j = i + 2;
        while (j < text.length && text[j] !== "\x07" && !(text[j] === "\x1b" && text[j + 1] === "\\")) j += 1;
        i = text[j] === "\x07" ? j + 1 : j + 2;
        continue;
      }
      i += 2;
      continue;
    }
    if (ch === "\r") { c = 0; i += 1; continue; }
    if (ch === "\n") {
      if (r >= rows - 1) {
        // **唯一与原实现不同之处**：滚出屏幕的顶行进入 scrollback（而不是被丢弃）。
        scrollback.push(screen[0].join("").replace(/\s+$/, ""));
        for (let y = 1; y < rows; y += 1) screen[y - 1] = screen[y].slice();
        screen[rows - 1] = Array.from({ length: cols }, () => " ");
      } else {
        r += 1;
      }
      c = 0;
      i += 1;
      continue;
    }
    if (ch === "\b") { c = Math.max(c - 1, 0); i += 1; continue; }
    if (ch === "\x07") { i += 1; continue; }
    if (ch === "\t") { c = Math.min(c + (8 - (c % 8)), cols - 1); i += 1; continue; }
    if (ch < " ") { i += 1; continue; }
    writeChar(ch);
    i += 1;
  }
  const screenLines = screen.map((line) => line.join("").replace(/\s+$/, ""));
  // W1.3：新增 `scrollCount` 字段（既有字段与输出不变）。
  return { scrollback, screenLines, clearsScreen, clearsScrollback, scrollCount: scrollback.length };
}

function frames(file, cols, rows) {
  const raw = fs.readFileSync(file);
  const metaPath = file.replace(/raw\.bin$/, "meta.json");
  const meta = fs.existsSync(metaPath) ? JSON.parse(fs.readFileSync(metaPath, "utf8")) : { snapshotOffsets: [] };
  const offsets = (meta.snapshotOffsets ?? []).length > 0 ? meta.snapshotOffsets.map((o) => o.offset) : [raw.length];
  const out = [];
  for (const offset of offsets) {
    const snapped = snapOffsetToFrameEnd(raw, offset) ?? offset;
    const full = replayFull(raw.subarray(0, snapped), cols, rows);
    // **不归一化**：行数统计必须用 renderScreen 的原始 rows 行（normalizeScreen 会折叠连续空行+掩码）。
    const visible = renderScreen(raw.subarray(0, snapped), cols, rows).map((l) => l.replace(/\s+$/, ""));
    const tailMatches = JSON.stringify(full.screenLines) === JSON.stringify(visible);
    const all = [...full.scrollback, ...full.screenLines];
    out.push({
      fullLines: all.filter((l) => l !== "").length,
      visibleLines: visible.filter((l) => l !== "").length,
      effectiveLines: visible.filter((l) => l.trim() !== "").length,
      clearsScreen: full.clearsScreen,
      clearsScrollback: full.clearsScrollback,
      tailMatches,
    });
  }
  return out;
}

function main() {
const args = process.argv.slice(2).filter((a) => a !== "--self-check");
if (args.length === 0) { console.error("用法: node tools/difftest/replay-scrollback.mjs [--self-check] <raw.bin>..."); process.exit(2); }
const rows = Number.parseInt(process.env.ROWS ?? "30", 10);
const cols = Number.parseInt(process.env.COLS ?? "100", 10);
for (const file of args) {
  const table = frames(file, cols, rows);
  console.log(`# ${file}（cols=${cols} rows=${rows}）`);
  table.forEach((f, idx) => console.log(`  帧${idx}: 全文行=${f.fullLines} 可见行=${f.visibleLines} 有效行=${f.effectiveLines} 2J=${f.clearsScreen} 3J=${f.clearsScrollback} 可见窗口与renderScreen一致=${f.tailMatches}`));
}
}

if (import.meta.url === `file://${process.argv[1]}`) main();
