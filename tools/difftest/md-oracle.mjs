// TS 侧 markdown 直渲染 oracle（F34/M1a；不走 pty，不新增依赖）。
//
// 用法：node tools/difftest/md-oracle.mjs --entry <pnpm cli.js> --theme dark|light --width 80 --in file.md
// 输出：每行一条带 ANSI 的渲染结果（`JSON.stringify` 逐行，便于稳定 diff）。
//
// 构造参数照 ref：`assistant-message.ts:114`
//   `new Markdown(content.text.trim(), outputPad=1, paddingY=0, markdownTheme)`
// 主题：`theme/theme.ts:1170` 的 `getMarkdownTheme()`，随当前主题名（dark/light）变化。
// chalk 在非 TTY 下自动关色，`theme.bold()` 会变成空操作 ⇒ 必须先强制真彩，
// 否则 oracle 输出的"无粗体"会污染整个对拍（实测 H1 少 `\x1b[1m`/`\x1b[4m`）。
process.env.FORCE_COLOR = "3";

import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const argv = process.argv.slice(2);
const get = (name, fallback) => {
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : fallback;
};

const entry = get("--entry", null);
const themeName = get("--theme", "dark");
const width = Number(get("--width", "80"));
const inputFile = get("--in", null);
// 批量模式：`--dir <目录>`，每个 `.md` 输出一条 `{"file":..., "lines":[...]}`（F35/G0 度量用）。
const inputDir = get("--dir", null);
if (!entry || (!inputFile && !inputDir)) {
  console.error(
    "用法: md-oracle.mjs --entry <pi cli.js> --theme dark|light --width N (--in file.md | --dir 目录)",
  );
  process.exit(2);
}

// `dist/bundle/cli.js` 的包根是 `dist/..`（`bundle` 上面一层），不是 `../../..`。
const pkgRoot = path.resolve(path.dirname(entry), "../..");
const loaded = await import(pathToFileURL(path.join(pkgRoot, "dist/index.js")).href);
const mod = loaded.default ?? loaded;
// `Markdown` 组件本体在 `@earendil-works/pi-tui`；主题函数在 coding-agent 包里。
const tuiLoaded = await import(
  pathToFileURL(
    path.join(pkgRoot, "..", "pi-tui", "dist/index.js"),
  ).href
).catch(async () => {
  // pnpm 布局：兄弟包在同一个 `.pnpm` 目录下
  const scope = path.resolve(pkgRoot, "..");
  const candidate = path.join(scope, "pi-tui/dist/index.js");
  return import(pathToFileURL(candidate).href);
});
const { Markdown } = tuiLoaded.default ?? tuiLoaded;
const { getMarkdownTheme, initTheme } = mod;
if (!Markdown || !getMarkdownTheme || !initTheme) {
  console.error("[md-oracle] 缺 Markdown/getMarkdownTheme/initTheme");
  process.exit(2);
}
initTheme(themeName);
const render = (text) => {
  const markdown = new Markdown(text.trim(), 1, 0, getMarkdownTheme());
  return markdown.render(width);
};
if (inputDir) {
  for (const name of fs.readdirSync(inputDir).filter((f) => f.endsWith(".md")).sort()) {
    const text = fs.readFileSync(path.join(inputDir, name), "utf8");
    console.log(JSON.stringify({ file: name, lines: render(text) }));
  }
} else {
  render(fs.readFileSync(inputFile, "utf8")).forEach((line) => console.log(JSON.stringify(line)));
}
