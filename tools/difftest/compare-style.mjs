// 逐单元格**样式**对拍（P8d/C2，新文件；不改既有比较工具的判据）。
//
// 现有 `compare-tui.mjs` 丢弃 SGR（其 `:4` 注释与 `:59` 实现），所以颜色/粗体差异看不见。
// 本工具把两侧 raw 解析成"屏幕单元格"：`{ch, fg, bg, bold, italic, dim, inverse}`，
// 逐帧逐单元格比较，输出首个差异位置与分类计数。
//
// 用法：
//   node tools/difftest/compare-style.mjs --left ts.raw --right rs.raw [--cols 100] [--rows 30] [--json]
import fs from "fs";

const DEFAULT = { fg: null, bg: null, bold: false, italic: false, dim: false, inverse: false };

function blank(cols) {
  return Array.from({ length: cols }, () => ({ ch: " ", ...DEFAULT }));
}

/** 把一段终端输出渲染成 `rows × cols` 的单元格矩阵（含样式）。 */
export function renderStyledScreen(text, cols, rows) {
  const screen = Array.from({ length: rows }, () => blank(cols));
  let r = 0;
  let c = 0;
  let style = { ...DEFAULT };
  const sgr = (params) => {
    const nums = params.length === 0 ? [0] : params;
    for (let i = 0; i < nums.length; i += 1) {
      const n = nums[i];
      if (n === 0) style = { ...DEFAULT };
      else if (n === 1) style.bold = true;
      else if (n === 2) style.dim = true;
      else if (n === 3) style.italic = true;
      else if (n === 7) style.inverse = true;
      else if (n === 22) {
        style.bold = false;
        style.dim = false;
      } else if (n === 23) style.italic = false;
      else if (n === 27) style.inverse = false;
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
  };
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (ch === "\x1b") {
      const next = text[i + 1];
      if (next === "[") {
        let j = i + 2;
        while (j < text.length && !/[@-~]/.test(text[j])) j += 1;
        const body = text.slice(i + 2, j);
        const final = text[j];
        const nums = body
          .replace(/^[?<>=]/, "")
          .split(";")
          .filter((part) => part !== "")
          .map((part) => Number.parseInt(part, 10))
          .filter((n) => Number.isFinite(n));
        if (final === "m") sgr(nums);
        else if (final === "H" || final === "f") {
          r = Math.max(0, (nums[0] ?? 1) - 1);
          c = Math.max(0, (nums[1] ?? 1) - 1);
        } else if (final === "A") r = Math.max(0, r - (nums[0] ?? 1));
        else if (final === "B") r = Math.min(rows - 1, r + (nums[0] ?? 1));
        else if (final === "C") c = Math.min(cols - 1, c + (nums[0] ?? 1));
        else if (final === "D") c = Math.max(0, c - (nums[0] ?? 1));
        else if (final === "K") {
          const mode = nums[0] ?? 0;
          if (mode === 0) for (let x = c; x < cols; x += 1) screen[r][x] = { ch: " ", ...DEFAULT };
          else if (mode === 2) screen[r].fill({ ch: " ", ...DEFAULT });
        } else if (final === "J") {
          const mode = nums[0] ?? 0;
          if (mode === 0) {
            for (let x = c; x < cols; x += 1) screen[r][x] = { ch: " ", ...DEFAULT };
            for (let y = r + 1; y < rows; y += 1) screen[y] = blank(cols).map((cell) => ({ ...cell }));
          } else if (mode === 2) {
            for (let y = 0; y < rows; y += 1) screen[y] = blank(cols).map((cell) => ({ ...cell }));
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
      c = 0;
      i += 1;
      continue;
    }
    if (ch === "\n") {
      r = Math.min(rows - 1, r + 1);
      i += 1;
      continue;
    }
    if (ch === "\b") {
      c = Math.max(0, c - 1);
      i += 1;
      continue;
    }
    if (ch === "\x07") {
      i += 1;
      continue;
    }
    if (r < rows && c < cols) {
      screen[r][c] = { ch, ...style };
    }
    c += 1;
    i += 1;
  }
  return screen;
}

/** 比较两个样式屏幕；返回首个差异与分类计数。 */
export function compareStyled(left, right, cols, rows, { styleOnly = false } = {}) {
  const a = renderStyledScreen(left, cols, rows);
  const b = renderStyledScreen(right, cols, rows);
  const counts = { char: 0, fg: 0, bg: 0, bold: 0, italic: 0, dim: 0, inverse: 0 };
  let first = null;
  for (let row = 0; row < rows; row += 1) {
    for (let col = 0; col < cols; col += 1) {
      const x = a[row][col];
      const y = b[row][col];
      const keys = styleOnly
        ? ["fg", "bg", "bold", "italic", "dim", "inverse"]
        : ["ch", "fg", "bg", "bold", "italic", "dim", "inverse"];
      const hit = keys.filter((key) => x[key] !== y[key]);
      if (hit.length === 0) continue;
      for (const key of hit) counts[key === "ch" ? "char" : key] += 1;
      if (!first) first = { row, col, left: x, right: y, fields: hit };
    }
  }
  const total = Object.values(counts).reduce((sum, n) => sum + n, 0);
  return { status: total === 0 ? "same" : "different", counts, first, total };
}

function main() {
  const argv = process.argv.slice(2);
  const get = (name, fallback) => {
    const index = argv.indexOf(name);
    return index >= 0 ? argv[index + 1] : fallback;
  };
  const leftPath = get("--left");
  const rightPath = get("--right");
  const cols = Number.parseInt(get("--cols", "100"), 10);
  const rows = Number.parseInt(get("--rows", "30"), 10);
  if (!leftPath || !rightPath) {
    console.error(
      "用法: node compare-style.mjs --left a.raw --right b.raw [--cols 100] [--rows 30] [--style-only] [--json]",
    );
    return 2;
  }
  const result = compareStyled(
    fs.readFileSync(leftPath, "utf8"),
    fs.readFileSync(rightPath, "utf8"),
    cols,
    rows,
    { styleOnly: argv.includes("--style-only") },
  );
  if (argv.includes("--json")) {
    console.log(JSON.stringify(result, null, 2));
  } else {
    console.log(`[compare-style] ${result.status}${result.status === "different" ? ` total=${result.total}` : ""}`);
    if (result.first) {
      const { row, col, left, right, fields } = result.first;
      console.log(`  首个差异 r${row} c${col} 字段=${fields.join(",")}`);
      console.log(`  TS : ${JSON.stringify(left)}`);
      console.log(`  rs : ${JSON.stringify(right)}`);
      console.log(`  分类计数: ${JSON.stringify(result.counts)}`);
    }
  }
  return result.status === "same" ? 0 : 1;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  process.exit(main());
}
