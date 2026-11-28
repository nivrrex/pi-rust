// F35/G0b：把两侧 ANSI 行展开成**逐单元格属性**并比较（修正 `md-diff` 的"分段方式"类误报）。
//
// 背景：TS 用 `\x1b[38;2;…m\x1b[1m…\x1b[24m\x1b[22m\x1b[39m` 这类**成对**开关，我们输出
// `\x1b[1;4;38;2;…m…\x1b[0m`。按字节比永远不等，但逐格属性可能完全相同。
// 本模块把一行展开成 `{char, fg, bg, bold, dim, italic, underline, strike}` 数组后再比。
const DEFAULT = {
  char: " ",
  fg: null,
  bg: null,
  bold: false,
  dim: false,
  italic: false,
  underline: false,
  strike: false,
};

/** ANSI 行 → 单元格数组（末尾空白裁掉）。 */
export function toCells(line) {
  const cells = [];
  let style = { ...DEFAULT, char: undefined };
  for (let index = 0; index < line.length; index += 1) {
    if (line[index] === "\u001b" && line[index + 1] === "[") {
      const end = line.indexOf("m", index);
      if (end > 0) {
        const params = line.slice(index + 2, end).split(";").filter((p) => p !== "");
        for (let i = 0; i < params.length; i += 1) {
          const n = Number(params[i]);
          if (n === 0) style = { ...DEFAULT, char: undefined };
          else if (n === 1) style.bold = true;
          else if (n === 2) style.dim = true;
          else if (n === 3) style.italic = true;
          else if (n === 4) style.underline = true;
          else if (n === 9) style.strike = true;
          else if (n === 22) style.bold = false;
          else if (n === 23) style.italic = false;
          else if (n === 24) style.underline = false;
          else if (n === 29) style.strike = false;
          else if (n === 39) style.fg = null;
          else if (n === 49) style.bg = null;
          else if (n === 38 && params[i + 1] === "2") {
            style.fg = `rgb(${params[i + 2]},${params[i + 3]},${params[i + 4]})`;
            i += 4;
          } else if (n === 48 && params[i + 1] === "2") {
            style.bg = `rgb(${params[i + 2]},${params[i + 3]},${params[i + 4]})`;
            i += 4;
          }
        }
        index = end;
        continue;
      }
    }
    cells.push({ ...style, char: line[index] });
  }
  while (cells.length > 0 && cells[cells.length - 1].char === " ") cells.pop();
  return cells;
}

const KEYS = ["char", "fg", "bg", "bold", "dim", "italic", "underline", "strike"];

/** 两行逐格比；返回 `{different, fields:{field:count}, first:{col,field,ts,rs}}`。 */
export function diffLines(tsLine, rsLine) {
  const a = toCells(tsLine);
  const b = toCells(rsLine);
  const fields = {};
  let different = 0;
  let first;
  const total = Math.max(a.length, b.length);
  for (let column = 0; column < total; column += 1) {
    const left = a[column] ?? DEFAULT;
    const right = b[column] ?? DEFAULT;
    const differing = KEYS.filter((key) => left[key] !== right[key]);
    if (differing.length === 0) continue;
    different += 1;
    for (const key of differing) fields[key] = (fields[key] ?? 0) + 1;
    if (!first) first = { col: column, field: differing.join(","), ts: left, rs: right };
  }
  return { different, fields, first };
}
