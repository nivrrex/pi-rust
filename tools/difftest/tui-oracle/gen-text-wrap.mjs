// tui-oracle：ref 真实 `Text` 类的换行/内边距夹具（P9-1-E4b / S3）。
//
// 用法：
//   node --import ./tools/difftest/tui-oracle/register.mjs \
//        tools/difftest/tui-oracle/gen-text-wrap.mjs \
//        tools/difftest/tui-oracle/fixtures-text-wrap.json
import fs from "node:fs";
import path from "node:path";

const REF = process.env.PI_TUI_REF_DIR
	? path.resolve(process.env.PI_TUI_REF_DIR, "..", "..")
	: path.resolve(path.dirname(new URL(import.meta.url).pathname), "../../../ref/pi-0.87.1/packages/coding-agent/src");
const textModule = path.resolve(REF, "../../tui/src/components/text.ts");
const { Text } = await import(textModule);

const cases = [
	{ name: "plain-short", text: "hello", paddingX: 1, paddingY: 0, width: 20 },
	{ name: "plain-long-20", text: "the quick brown fox jumps over the lazy dog", paddingX: 1, paddingY: 0, width: 20 },
	{ name: "plain-long-40", text: "the quick brown fox jumps over the lazy dog", paddingX: 1, paddingY: 0, width: 40 },
	{ name: "wide-cjk-20", text: "中文宽字符测试中文宽字符测试", paddingX: 1, paddingY: 0, width: 20 },
	{ name: "ansi-color-20", text: "\u001b[31mred part\u001b[0m and normal", paddingX: 1, paddingY: 0, width: 20 },
	{ name: "tabs", text: "a\tb\tc", paddingX: 1, paddingY: 0, width: 40 },
	{ name: "padding-y", text: "padded", paddingX: 1, paddingY: 1, width: 40 },
	{ name: "padding-x2-narrow", text: "abcdef", paddingX: 2, paddingY: 0, width: 5 },
	{ name: "empty", text: "", paddingX: 1, paddingY: 1, width: 20 },
];

const out = cases.map((c) => {
	const component = new Text(c.text, c.paddingX, c.paddingY);
	return {
		name: c.name,
		width: c.width,
		text: c.text,
		paddingX: c.paddingX,
		paddingY: c.paddingY,
		lines: component.render(c.width),
	};
});

const target = process.argv[2];
fs.writeFileSync(target, JSON.stringify({ generatedBy: "gen-text-wrap.mjs", cases: out }, null, 2));
fs.writeFileSync(
	target.replace(/\.json$/, ".lines.txt"),
	out.map((entry) => `${entry.name} @${entry.width}\n${entry.lines.join("\n")}\n`).join("====\n"),
);
console.log(`wrote ${out.length} cases -> ${target}`);
