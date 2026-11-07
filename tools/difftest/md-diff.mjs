// F34/M1b：pi-rs（cargo example）与 TS（md-oracle）的 markdown 渲染逐行/逐格对拍。
//
// 用法：node tools/difftest/md-diff.mjs --entry <pi cli.js> --theme dark --width 80 --in file.md
// 输出：首个差异的位置与两侧文本；`same` 时退出码 0。
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const argv = process.argv.slice(2);
const get = (name, fallback) => {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : fallback;
};
const entry = get("--entry", null);
const theme = get("--theme", "dark");
const width = get("--width", "80");
const input = get("--in", null);
// TS 的 `Markdown.render(width)` 与 TUI 内实际渲染在**空行**上不同（paddingY/裁切层面），
// 而空行语义已由 TUI golden（`T05` 等）锁定、不能改。分类时用 `--ignore-blank` 只比内容行。
const ignoreBlank = argv.includes("--ignore-blank");
const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../..");

if (!entry || !input) {
  console.error("用法: md-diff.mjs --entry <cli.js> --theme dark|light --width N --in file.md");
  process.exit(2);
}

const tsRaw = execFileSync("node", [
  path.join(repo, "tools/difftest/md-oracle.mjs"),
  "--entry", entry, "--theme", theme, "--width", width, "--in", input,
], { encoding: "utf8" }).trim().split("\n").map((line) => JSON.parse(line));
const rsRaw = execFileSync("cargo", [
  "run", "-q", "-p", "pi-tui", "--example", "md-render", "--offline", "--",
  "--theme", theme, "--width", width, input,
], { encoding: "utf8", cwd: repo, stdio: ["ignore", "pipe", "ignore"] })
  .trim().split("\n").map((line) => JSON.parse(line));

// SGR 归一：TS 用 `\x1b[39m`（仅重置前景）与 `\x1b[0m`，我们统一 `\x1b[0m`；比较前剥掉所有 SGR 与尾随空格。
const strip = (text) => text.replace(/\u001b\[[0-9;]*m/g, "").replace(/\s+$/, "");
// SGR 归一成"逐列属性集"：TS 用 `\x1b[1m…\x1b[22m` 成对开关、我们用 `\x1b[1;38;2;…m…\x1b[0m`，
// 直接比字节永远不等。这里把每行展开成 `{char, bold, italic, underline, strike, fg}` 序列后比较。
function cells(text) {
  const out = [];
  let style = { bold: false, italic: false, underline: false, strike: false, fg: null };
  for (let index = 0; index < text.length; index += 1) {
    if (text[index] === "\u001b" && text[index + 1] === "[") {
      const end = text.indexOf("m", index);
      if (end > 0) {
        const params = text.slice(index + 2, end).split(";").filter((p) => p !== "");
        for (let i = 0; i < params.length; i += 1) {
          const n = Number(params[i]);
          if (n === 0) style = { bold: false, italic: false, underline: false, strike: false, fg: null };
          else if (n === 1) style.bold = true;
          else if (n === 22) style.bold = false;
          else if (n === 3) style.italic = true;
          else if (n === 23) style.italic = false;
          else if (n === 4) style.underline = true;
          else if (n === 24) style.underline = false;
          else if (n === 9) style.strike = true;
          else if (n === 29) style.strike = false;
          else if (n === 39) style.fg = null;
          else if (n === 38 && params[i + 1] === "2") {
            style.fg = `rgb(${params[i + 2]},${params[i + 3]},${params[i + 4]})`;
            i += 4;
          }
        }
        index = end;
        continue;
      }
    }
    out.push({ char: text[index], ...style });
  }
  // 去掉行尾空白单元（TS 会把整行 padding 补满宽度）。
  while (out.length > 0 && out[out.length - 1].char === " ") out.pop();
  return out;
}
const sgrOnly = (text) => JSON.stringify(cells(text));

const ts = ignoreBlank ? tsRaw.filter((line) => strip(line) !== "") : tsRaw;
const rs = ignoreBlank ? rsRaw.filter((line) => strip(line) !== "") : rsRaw;
let differences = 0;
for (let index = 0; index < Math.max(ts.length, rs.length); index += 1) {
  const a = ts[index] ?? "";
  const b = rs[index] ?? "";
  if (a === b) continue;
  const textSame = strip(a) === strip(b);
  if (textSame && sgrOnly(a) === sgrOnly(b)) continue;
  differences += 1;
  if (differences <= 12) {
    console.log(`--- r${index} ${textSame ? "(仅样式)" : "(文本)"}`);
    console.log(`TS ${JSON.stringify(a)}`);
    console.log(`RS ${JSON.stringify(b)}`);
  }
}
console.log(
  `[md-diff] lines ts=${ts.length} rs=${rs.length} different=${differences}${ignoreBlank ? " (ignore-blank)" : ""}`,
);
process.exit(differences === 0 ? 0 : 1);
