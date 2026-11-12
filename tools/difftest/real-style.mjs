#!/usr/bin/env node
// F54/E：**按步**逐单元格样式对拍（诊断工具，不是闸门）。
//
// 为什么另开一个文件：`compare-style.mjs` 曾因"88 全假差异"被降级，它的
// `compareStyled` 整段流回放到底、两侧帧数一多就对不上。这里改成与
// `compare-steps.py` 完全相同的**步切片**口径：用 `marks.jsonl` 的字节偏移截断 raw，
// 再用 `compare-style.mjs` 导出的 `renderStyledScreen`（不改它）渲染成带样式的屏幕，
// 逐单元格比较，并按**行内容类别**归类差异，供人先看分类再决定哪些进验收。
//
// 用法：
//   node tools/difftest/real-style.mjs --left DIR --right DIR --cols 110 --rows 40 \
//       [--steps N] [--json]
//
// `DIR` 是 `run-real.sh` 的输出目录（含 `raw.bin` 与 `steps/marks.jsonl`）。

import fs from "node:fs";
import path from "node:path";
import { renderStyledScreen, frameEnds } from "./compare-style.mjs";
import { trimRawToOffset } from "./compare-tui.mjs";

const argv = process.argv.slice(2);
const get = (name, fallback) => {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : fallback;
};

const leftDir = get("--left");
const rightDir = get("--right");
const cols = Number.parseInt(get("--cols", "110"), 10);
const rows = Number.parseInt(get("--rows", "40"), 10);
const maxSteps = get("--steps") ? Number.parseInt(get("--steps"), 10) : Number.POSITIVE_INFINITY;
const asJson = argv.includes("--json");
if (!leftDir || !rightDir) {
  console.error("用法: real-style.mjs --left DIR --right DIR --cols N --rows N [--steps N] [--json]");
  process.exit(2);
}

/** 读 `steps/marks.jsonl`；pi-rs 侧由钩子写、TS 侧由 pty-helper 写，字段名都是 `offset`。 */
function readMarks(dir) {
  // pi-rs 侧钩子把 mark 写进 `steps/marks.jsonl`，TS 侧 pty-helper 写 `marks.jsonl`（目录根）。
  const candidates = [path.join(dir, "marks.jsonl"), path.join(dir, "steps", "marks.jsonl")];
  // pi-rs 侧还会由 pty-helper 创建一个**空**的根 `marks.jsonl`，必须跳过它挑非空的那份。
  const file = candidates.find((candidate) => {
    try {
      return fs.statSync(candidate).size > 0;
    } catch {
      return false;
    }
  });
  if (!file) return [];
  return fs
    .readFileSync(file, "utf8")
    .split("\n")
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line));
}

/**
 * 行类别（只按**可见文本**判断，样式不参与）：
 * 这些前缀取自 ref 的各渲染器（`renderers/*.ts`）与 `footer.ts`，是稳定可辨的形态。
 */
function classify(rowText) {
  const text = rowText.trimEnd();
  const trimmed = text.trimStart();
  if (trimmed === "") return "blank";
  if (/^[─]{10,}$/.test(trimmed)) return "frame-border";
  if (/^(↑|↓|R\d|W\d|CH[\d.]+%)/.test(trimmed) || /\(auto\)/.test(trimmed)) return "footer-stats";
  if (/^[✓✗·]/.test(trimmed) || /^ (read|write|edit|ls|grep|find|bash|\$|cg_recall|todo|report_to_user)\b/.test(text)) {
    return "tool-row";
  }
  if (/^[+-]\s*\d+\s/.test(trimmed)) return "diff-line";
  if (/^(Thinking|\+ Thought|− Thought|Thinking\.\.\.)/.test(trimmed)) return "thinking";
  if (/^[☐☑☒]/.test(trimmed) || /^\d+\.\s/.test(trimmed)) return "todo-or-list";
  if (/^[│┌┐└┘├┤┬┴┼]/.test(trimmed) || /^\|/.test(trimmed)) return "table";
  if (/^#/.test(trimmed) || /^\*\*/.test(trimmed) || /^>/.test(trimmed)) return "markdown";
  return "body";
}

function styleDiff(a, b) {
  const fields = [];
  for (const key of ["fg", "bg", "bold", "italic", "dim", "inverse"]) {
    if (JSON.stringify(a[key]) !== JSON.stringify(b[key])) fields.push(key);
  }
  return fields;
}

function compareStep(leftRaw, rightRaw, leftOffset, rightOffset) {
  const leftScreen = renderStyledScreen(
    trimRawToOffset(leftRaw, leftOffset).toString("utf8"),
    cols,
    rows,
  );
  const rightScreen = renderStyledScreen(
    trimRawToOffset(rightRaw, rightOffset).toString("utf8"),
    cols,
    rows,
  );
  const categories = new Map();
  let total = 0;
  for (let r = 0; r < rows; r += 1) {
    const text = leftScreen[r].map((cell) => cell.ch).join("");
    const kind = classify(text);
    for (let c = 0; c < cols; c += 1) {
      const l = leftScreen[r][c];
      const rr = rightScreen[r][c];
      if (l.ch !== rr.ch) {
        // 文本不同不是样式差异（文本差异交给 `compare-steps`）⇒ 只登记为 text 类。
        const entry = categories.get("text") ?? { cells: 0, first: null };
        entry.cells += 1;
        if (!entry.first) entry.first = { row: r, col: c, ref: l.ch, ours: rr.ch };
        categories.set("text", entry);
        continue;
      }
      const fields = styleDiff(l, rr);
      if (fields.length === 0) continue;
      total += 1;
      const entry = categories.get(kind) ?? { cells: 0, fields: new Map(), first: null };
      entry.cells += 1;
      for (const field of fields) entry.fields.set(field, (entry.fields.get(field) ?? 0) + 1);
      if (!entry.first) {
        entry.first = {
          row: r,
          col: c,
          ch: l.ch,
          ref: fields.map((f) => `${f}=${JSON.stringify(l[f])}`).join(" "),
          ours: fields.map((f) => `${f}=${JSON.stringify(rr[f])}`).join(" "),
          line: text.trimEnd().slice(0, 60),
        };
      }
      categories.set(kind, entry);
    }
  }
  return { total, categories };
}

const leftRaw = fs.readFileSync(path.join(leftDir, "raw.bin"));
const rightRaw = fs.readFileSync(path.join(rightDir, "raw.bin"));
const leftMarks = readMarks(leftDir);
const rightMarks = readMarks(rightDir);
const steps = Math.min(leftMarks.length, rightMarks.length, maxSteps);
if (steps === 0) {
  console.error("real-style: 至少一侧没有 marks.jsonl（先跑 run-real.sh）");
  process.exit(2);
}

const report = { cols, rows, steps: [], frames: {} };
report.frames.left = frameEnds(leftRaw.toString("utf8")).length;
report.frames.right = frameEnds(rightRaw.toString("utf8")).length;

for (let index = 0; index < steps; index += 1) {
  const { total, categories } = compareStep(
    leftRaw,
    rightRaw,
    leftMarks[index].offset,
    rightMarks[index].offset,
  );
  const rendered = [...categories.entries()]
    .filter(([kind]) => kind !== "text")
    .map(([kind, entry]) => ({
      kind,
      cells: entry.cells,
      fields: Object.fromEntries(entry.fields),
      first: entry.first,
    }))
    .sort((a, b) => b.cells - a.cells);
  report.steps.push({ step: index, cells: total, categories: rendered });
}

if (asJson) {
  console.log(JSON.stringify(report, null, 2));
} else {
  console.log(`real-style: ${cols}x${rows} 帧数 ref=${report.frames.left} ours=${report.frames.right}`);
  let grand = 0;
  for (const step of report.steps) {
    grand += step.cells;
    console.log(`step-${step.step}: 样式差异单元格 ${step.cells}`);
    for (const item of step.categories) {
      const fields = Object.entries(item.fields)
        .map(([name, count]) => `${name}×${count}`)
        .join(" ");
      console.log(`  ${item.kind.padEnd(12)} ${String(item.cells).padStart(5)}  ${fields}`);
      if (item.first) {
        console.log(
          `    首次 r${item.first.row}c${item.first.col} ref=${item.first.ref} ours=${item.first.ours} | ${item.first.line}`,
        );
      }
    }
  }
  console.log(`real-style: 合计样式差异单元格 ${grand}`);
}
process.exit(0);
