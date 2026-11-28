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
import { frameEnds } from "./compare-style.mjs";
import { trimRawToOffset } from "./compare-tui.mjs";

// F57/B3：`compare-style.mjs` 的 SGR 解析只认 0/1/2/3/7/22/23/27/39/49/38/48 与
// 30–37/90–97/40–47/100–107，**没有 underline(4/24) 与 strike(9/29)**。
// 那个文件是红线（不得改），所以这里自带一个解析器，覆盖**全部**会被主题用到的属性：
// fg / bg / bold / dim / italic / underline / strike / inverse。
// 与 `compare-style` 的差异只在这里，屏幕模型（CSI 光标、换行滚动、宽字符）仍复用
// `compare-tui.mjs` 的口径（`renderStyledScreen` 不复用，因为它缺属性）。
const DEFAULT = {
  fg: null,
  bg: null,
  bold: false,
  dim: false,
  italic: false,
  underline: false,
  strike: false,
  inverse: false,
};

function applySgr(style, params) {
  const nums = params.length === 0 ? [0] : params;
  for (let i = 0; i < nums.length; i += 1) {
    const n = nums[i];
    if (n === 0) Object.assign(style, DEFAULT);
    else if (n === 1) style.bold = true;
    else if (n === 2) style.dim = true;
    else if (n === 3) style.italic = true;
    else if (n === 4) style.underline = true;
    else if (n === 7) style.inverse = true;
    else if (n === 9) style.strike = true;
    else if (n === 21 || n === 22) {
      if (n === 22) {
        style.bold = false;
        style.dim = false;
      } else style.bold = false;
    } else if (n === 23) style.italic = false;
    else if (n === 24) style.underline = false;
    else if (n === 27) style.inverse = false;
    else if (n === 29) style.strike = false;
    else if (n === 39) style.fg = null;
    else if (n === 49) style.bg = null;
    else if (n === 38 || n === 48) {
      const key = n === 38 ? "fg" : "bg";
      const mode = nums[i + 1];
      if (mode === 5) {
        style[key] = `i${nums[i + 2]}`;
        i += 2;
      } else if (mode === 2) {
        style[key] = `rgb(${nums[i + 2]},${nums[i + 3]},${nums[i + 4]})`;
        i += 4;
      }
    } else if (n >= 30 && n <= 37) style.fg = `a${n - 30}`;
    else if (n >= 90 && n <= 97) style.fg = `a${n - 90 + 8}`;
    else if (n >= 40 && n <= 47) style.bg = `a${n - 40}`;
    else if (n >= 100 && n <= 107) style.bg = `a${n - 100 + 8}`;
  }
}

/** 把一段终端字节渲染成 `rows × cols` 的单元格矩阵（含**全部** SGR 属性）。 */
function renderStyledFull(text, width, height) {
  const screen = Array.from({ length: height }, () =>
    Array.from({ length: width }, () => ({ ch: " ", ...DEFAULT })),
  );
  let row = 0;
  let col = 0;
  let style = { ...DEFAULT };
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (ch === "\x1b") {
      const next = text[i + 1];
      if (next === "[") {
        let j = i + 2;
        while (j < text.length && !/[A-Za-z@]/.test(text[j])) j += 1;
        const final = text[j];
        const body = text.slice(i + 2, j);
        if (final === "m") {
          applySgr(style, body.split(";").map((part) => (part === "" ? 0 : Number.parseInt(part, 10) || 0)));
        } else if (final === "H" || final === "f") {
          const [r = 1, c = 1] = body.split(";").map((p) => Number.parseInt(p, 10) || 1);
          row = Math.min(height - 1, Math.max(0, r - 1));
          col = Math.min(width - 1, Math.max(0, c - 1));
        } else if (final === "A") row = Math.max(0, row - (Number.parseInt(body, 10) || 1));
        else if (final === "B") row = Math.min(height - 1, row + (Number.parseInt(body, 10) || 1));
        else if (final === "C") col = Math.min(width - 1, col + (Number.parseInt(body, 10) || 1));
        else if (final === "D") col = Math.max(0, col - (Number.parseInt(body, 10) || 1));
        else if (final === "G") col = Math.min(width - 1, Math.max(0, (Number.parseInt(body, 10) || 1) - 1));
        else if (final === "K") {
          const mode = Number.parseInt(body, 10) || 0;
          if (mode === 0) for (let x = col; x < width; x += 1) screen[row][x] = { ch: " ", ...DEFAULT };
          else if (mode === 2) screen[row].fill({ ch: " ", ...DEFAULT });
        } else if (final === "J") {
          const mode = Number.parseInt(body, 10) || 0;
          if (mode === 2 || mode === 3) {
            for (let y = 0; y < height; y += 1) screen[y].fill({ ch: " ", ...DEFAULT });
            row = 0;
            col = 0;
          }
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
    if (ch === "\r") {
      col = 0;
      i += 1;
      continue;
    }
    if (ch === "\n") {
      if (row >= height - 1) {
        for (let y = 1; y < height; y += 1) screen[y - 1] = screen[y].slice();
        screen[height - 1] = Array.from({ length: width }, () => ({ ch: " ", ...DEFAULT }));
      } else row += 1;
      col = 0;
      i += 1;
      continue;
    }
    if (ch === "\b") {
      col = Math.max(0, col - 1);
      i += 1;
      continue;
    }
    if (ch === "\x07") {
      i += 1;
      continue;
    }
    if (ch < " ") {
      i += 1;
      continue;
    }
    const target = col >= width ? width - 1 : col;
    screen[row][target] = { ch, ...style };
    col += 1;
    i += 1;
  }
  return screen;
}

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
  // F57/B3：比较**全部**属性（原缺 underline / strike）。
  for (const key of ["fg", "bg", "bold", "dim", "italic", "underline", "strike", "inverse"]) {
    if (JSON.stringify(a[key]) !== JSON.stringify(b[key])) fields.push(key);
  }
  return fields;
}

function compareStep(leftRaw, rightRaw, leftOffset, rightOffset) {
  const leftScreen = renderStyledFull(
    trimRawToOffset(leftRaw, leftOffset).toString("utf8"),
    cols,
    rows,
  );
  const rightScreen = renderStyledFull(
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
