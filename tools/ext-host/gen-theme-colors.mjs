// 由 ref 的 `theme/{dark,light}.json` 生成 `shim/theme-colors.mjs`（**不要手改产物**）。
//
// 为什么要两份：扩展的 `ctx.ui.theme.fg(...)` 必须跟随**当前主题**（`settings.json` 的
// `theme`）。此前产物只由 `dark.json` 生成 ⇒ light 主题下扩展仍拿深色调色板，
// 真机表现为 `✓ 全部完成` 在浅色主题里是深色的 `#b5bd68` 而不是浅色主题的 `#588458`
// （ref `theme/theme.ts:529` 按主题名取对应主题对象）。
//
// 用法：node tools/ext-host/gen-theme-colors.mjs
import fs from "node:fs";
import path from "node:path";

const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../..");
const themeDir = path.join(repo, "ref/pi-0.87.1/packages/coding-agent/src/modes/interactive/theme");

/** 读一份主题，把 `colors` 里的引用解析成 `vars` 的具体色值（只保留 `#rrggbb`）。 */
function loadTheme(name) {
  const data = JSON.parse(fs.readFileSync(path.join(themeDir, `${name}.json`), "utf8"));
  const vars = data.vars ?? {};
  const out = {};
  for (const [key, ref] of Object.entries(data.colors ?? {})) {
    const value = vars[String(ref).replace(/^\$/, "")] ?? ref;
    if (typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value)) {
      out[key] = [
        Number.parseInt(value.slice(1, 3), 16),
        Number.parseInt(value.slice(3, 5), 16),
        Number.parseInt(value.slice(5, 7), 16),
      ];
    }
  }
  return out;
}

const dark = loadTheme("dark");
const light = loadTheme("light");
const keys = Object.keys(dark);

const fg = (rgb) => `\\u001b[38;2;${rgb[0]};${rgb[1]};${rgb[2]}m`;
const bg = (rgb) => `\\u001b[48;2;${rgb[0]};${rgb[1]};${rgb[2]}m`;

const lines = [];
lines.push("// 由 ref `theme/dark.json` 与 `theme/light.json` 生成（含 `vars` 解析）。不要手改。");
lines.push("// 重新生成：`node tools/ext-host/gen-theme-colors.mjs`。");
lines.push("export const FG_COLORS = {");
for (const key of keys) lines.push(`  ${JSON.stringify(key)}: "${fg(dark[key])}",`);
lines.push("};");
lines.push("");
lines.push("export const BG_COLORS = {");
for (const key of keys) lines.push(`  ${JSON.stringify(key)}: "${bg(dark[key])}",`);
lines.push("};");
lines.push("");
lines.push("// 浅色主题（`settings.json` 的 `theme: \"light\"`）。键集与深色一致。");
lines.push("export const FG_COLORS_LIGHT = {");
for (const key of keys) lines.push(`  ${JSON.stringify(key)}: "${fg(light[key] ?? dark[key])}",`);
lines.push("};");
lines.push("");
lines.push("export const BG_COLORS_LIGHT = {");
for (const key of keys) lines.push(`  ${JSON.stringify(key)}: "${bg(light[key] ?? dark[key])}",`);
lines.push("};");
lines.push("");
lines.push("export const RESET_FG = \"\\u001b[39m\";");
lines.push("");
lines.push("/** 按主题名取前景色表（未知主题名按 dark，与 ref 的默认一致）。 */");
lines.push("export function fgColorsFor(themeName) {");
lines.push("  return themeName === \"light\" ? FG_COLORS_LIGHT : FG_COLORS;");
lines.push("}");
lines.push("");

fs.writeFileSync(path.join(repo, "tools/ext-host/shim/theme-colors.mjs"), lines.join("\n"));
console.log(`wrote ${keys.length} colors for dark + light`);
