// 逐格样式 dump（新文件；只 import 既有终端模拟器，**不改** compare-*）。
//
// 背景：`compare-style.mjs` 直接复用 `renderStyledScreen`，但在含 OSC-8 超链接的 raw 上出现
// `ch` 类差异（例：`T100-skills` r10c0 `' '` vs `'─'`，而 `compare-tui` 判 same）。本工具把同一
// 模拟器的结果**按行导出成稳定文本**，便于用 diff 定位到底是"字符错位"还是"样式不同"。
//
// 用法：
//   node tools/difftest/color-dump.mjs --raw a.bin --cols 100 --rows 30
//   node tools/difftest/color-dump.mjs --left a.bin --right b.bin --cols 100 --rows 30
import fs from "node:fs";
import { renderStyledScreen } from "./compare-style.mjs";

const argv = process.argv.slice(2);
const get = (name, fallback) => {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : fallback;
};

/** 把一帧渲染成每行一条的稳定文本：`行号│字符网格│样式段`。 */
export function dumpStyled(path, cols, rows) {
  const screen = renderStyledScreen(fs.readFileSync(path, "utf8"), cols, rows);
  const lines = [];
  screen.forEach((row, index) => {
    const text = row.map((cell) => cell.ch).join("").replace(/\s+$/, "");
    // 样式段：连续相同样式合并成 `列区间:fg/bg/属性`，空行只记 "-"。
    const runs = [];
    let start = 0;
    let current = null;
    const key = (cell) =>
      [cell.fg ?? "-", cell.bg ?? "-", cell.bold ? "b" : "", cell.dim ? "d" : "", cell.italic ? "i" : "", cell.inverse ? "v" : ""]
        .filter(Boolean)
        .join("/");
    for (let column = 0; column <= row.length; column += 1) {
      const next = column < row.length ? key(row[column]) : null;
      if (next === current) continue;
      if (current !== null && current !== "-") runs.push(`${start}-${column - 1}:${current}`);
      current = next;
      start = column;
    }
    lines.push(`${String(index).padStart(3, "0")}│${text}│${runs.join(" ") || "-"}`);
  });
  return lines;
}

function main() {
  const cols = Number(get("--cols", "100"));
  const rows = Number(get("--rows", "30"));
  const raw = get("--raw", null);
  const left = get("--left", null);
  const right = get("--right", null);
  if (raw) {
    for (const line of dumpStyled(raw, cols, rows)) console.log(line);
    return;
  }
  if (!left || !right) {
    console.error("用法: color-dump.mjs --raw a.bin | --left a.bin --right b.bin --cols N --rows N");
    process.exit(2);
  }
  const a = dumpStyled(left, cols, rows);
  const b = dumpStyled(right, cols, rows);
  let differences = 0;
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    if (a[index] === b[index]) continue;
    differences += 1;
    if (differences <= 10) {
      console.log(`--- r${index}`);
      console.log(`TS ${a[index] ?? ""}`);
      console.log(`RS ${b[index] ?? ""}`);
    }
  }
  console.log(`[color-dump] rows=${a.length} different_rows=${differences}`);
  process.exit(differences === 0 ? 0 : 1);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
