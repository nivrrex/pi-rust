// P7-6r2/W2：把宽度夹具转成 **Rust 常量表**（不引入 JSON 解析依赖、不做构建期代码生成）。
// 产物：`crates/pi-tui/src/hotkeys_widths.rs`（入库，随夹具一起更新）。
//
// 面板行构成（与现有 `HOTKEYS_LINES` 一致）：`[header, ""] + Markdown 块去掉尾部 paddingY 空行`。
import fs from "node:fs";
import path from "node:path";

const here = import.meta.dirname;
const fixturePath = path.join(here, "fixtures-width.json");
const outPath = process.argv[2] ?? path.join(here, "../../../crates/pi-tui/src/hotkeys_widths.rs");
const data = JSON.parse(fs.readFileSync(fixturePath, "utf8"));
const panel = data.panels.hotkeys;

const escape = (line) => line.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
const widths = data.widths;
const blocks = widths.map((width) => {
	const entry = panel[String(width)];
	const plain = entry.plain.map((l) => l.replace(/\s+$/, ""));
	// 去掉 Markdown 尾部 paddingY 空行（最后一个空行）
	while (plain.length > 0 && plain[plain.length - 1] === "") plain.pop();
	const lines = [" Keyboard Shortcuts", "", ...plain];
	return { width, lines };
});

const body = blocks
	.map(({ width, lines }) => `    (\n        ${width},\n        &[\n${lines.map((l) => `            "${escape(l)}",`).join("\n")}\n        ],\n    ),`)
	.join("\n");

const out = `//! **自动生成，请勿手改** —— 由 \`tools/difftest/tui-oracle/gen-hotkeys-rs.mjs\` 从
//! \`fixtures-width.json\` 生成（该夹具由真实 TS \`Markdown\` 组件渲染，见 \`gen-width.mjs\`）。
//!
//! 每档内容 = \`[header, ""] + Markdown 块\`（去掉尾部 paddingY 空行），与旧常量 \`HOTKEYS_LINES\`
//! 的形状一致；cols=100 档与旧常量逐字节相同（单测保证）。

/// \`(cols, 面板行)\`：仅含夹具覆盖的精确宽度档（60/80/100/120）。
pub(super) const HOTKEYS_PANEL_BY_WIDTH: &[(usize, &[&str])] = &[
${body}
];
`;

fs.writeFileSync(outPath, out);
console.log(`已生成 ${outPath}：${widths.length} 档，行数 ${blocks.map((b) => b.lines.length).join("/")}，大小 ${(fs.statSync(outPath).size / 1024).toFixed(1)} KB`);
