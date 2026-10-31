// edit 工具的 oracle：调用 TS 包里**真实的** `createEditToolDefinition`，把每个用例的
// 输入（文件内容 + edits）与输出（isError / 结果文本 / 改后文件内容 / diff）落成 JSON，
// 供 Rust 侧逐字节比对。
//
// 为什么不是纯函数 oracle：`edit-diff.ts` 只导出了 `generateDiffString` /
// `generateUnifiedPatch`，`fuzzyFindText`、`normalizeForFuzzyMatch`、
// `applyReplacementsPreservingUnchanged` 都是模块内部函数。要走真实实现，只能经
// 工具定义的 `execute()`。
//
// 用法见 tools/oracle/run-edit.sh。输出：fixtures/edit/cases.json

import { mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

const INDEX = process.env.PI_CODING_AGENT_INDEX;
if (!INDEX) {
	console.error("必须设置 PI_CODING_AGENT_INDEX 指向包的 dist/index.js");
	process.exit(2);
}
const { createEditToolDefinition } = await import(INDEX);

const E = (oldText, newText) => ({ oldText, newText });

/** 用例：`name` + `content`（初始文件）+ `edits`。 */
const CASES = [
	{ name: "exact_single_line", content: "alpha\nbeta\n", edits: [E("beta", "BETA")] },
	{ name: "exact_multi_line", content: "a\nb\nc\n", edits: [E("a\nb", "a\nB")] },
	{ name: "exact_first_line", content: "one\ntwo\n", edits: [E("one", "1")] },
	{ name: "exact_last_line_no_newline", content: "only", edits: [E("only", "ONLY")] },
	{ name: "two_disjoint_edits", content: "a\nb\nc\nd\n", edits: [E("a", "A"), E("d", "D")] },
	{ name: "edits_reversed_order", content: "a\nb\nc\n", edits: [E("c", "C"), E("a", "A")] },
	{
		name: "two_edits_same_line",
		content: "hello world\n",
		edits: [E("hello", "HELLO"), E("world", "WORLD")],
	},
	{ name: "duplicate_old_text", content: "same\nsame\n", edits: [E("same", "X")] },
	{
		name: "duplicate_only_by_trailing_space",
		content: "same\nsame  \n",
		edits: [E("same", "X")],
	},
	{ name: "missing_old_text", content: "alpha\n", edits: [E("nope", "x")] },
	{ name: "empty_old_text", content: "alpha\n", edits: [E("", "x")] },
	{ name: "no_change", content: "alpha\n", edits: [E("alpha", "alpha")] },
	// —— 模糊匹配：精确匹配失败，走归一化空间 ——
	{
		name: "fuzzy_trailing_ws_in_file",
		content: "alpha   \nbeta\n",
		edits: [E("alpha\nbeta", "ALPHA\nBETA")],
	},
	{
		name: "fuzzy_trailing_ws_in_old_text",
		content: "alpha\nbeta\n",
		edits: [E("alpha   \nbeta", "ALPHA\nBETA")],
	},
	{
		name: "fuzzy_leading_indent_change",
		content: "    indented\nnext\n",
		edits: [E("\tindented\nnext", "flat\nNEXT")],
	},
	{
		name: "fuzzy_untouched_line_keeps_trailing_ws",
		content: "alpha   \nbeta\ngamma   \n",
		edits: [E("alpha\nbeta", "ALPHA\nBETA")],
	},
	{ name: "fuzzy_single_line_trailing_ws", content: "x   \n", edits: [E("x", "Y")] },
	// —— 部分行覆盖：整行粒度 vs 字节偏移的关键差异场景 ——
	{
		name: "partial_line_prefix_and_suffix",
		content: "prefix MIDDLE suffix\n",
		edits: [E("MIDDLE", "CENTER")],
	},
	{
		name: "partial_line_with_trailing_ws_on_line",
		content: "aa bb cc   \n",
		edits: [E("bb", "BB")],
	},
	{
		name: "partial_line_with_case_diff",
		content: "alpha BETA gamma\n",
		edits: [E("beta", "X")],
	},
	// —— 行尾风格 ——
	{ name: "crlf_file_lf_old_text", content: "a\r\nb\r\n", edits: [E("a\nb", "A\nB")] },
	{ name: "crlf_file_crlf_old_text", content: "a\r\nb\r\n", edits: [E("a\r\nb", "A\r\nB")] },
	{ name: "crlf_fuzzy_keeps_crlf", content: "a   \r\nb\r\n", edits: [E("a\nb", "A\nB")] },
	{ name: "lf_file_old_text_with_crlf", content: "a\nb\n", edits: [E("a\r\nb", "A\nB")] },
	// —— 文本构造边界 ——
	{ name: "trailing_newline_removed", content: "a\nb\n", edits: [E("a\nb\n", "a\nb")] },
	{ name: "whole_file", content: "a\nb\n", edits: [E("a\nb\n", "c\n")] },
	{ name: "empty_file_edit", content: "", edits: [E("a", "b")] },
	{ name: "add_lines", content: "a\nb\n", edits: [E("b", "b\nc\nd")] },
	{ name: "unicode_content", content: "中文行一\n中文行二\n", edits: [E("中文行二", "替换")] },
	{
		name: "unicode_with_trailing_ws",
		content: "中文   \n第二行\n",
		edits: [E("中文\n第二行", "X\nY")],
	},
	// NFKC：TS 第一步是 `text.normalize("NFKC")`，全角应被折成半角。
	{ name: "nfkc_fullwidth_vs_halfwidth", content: "ＡＢＣ\n", edits: [E("ABC", "X")] },
	{ name: "nfkc_fullwidth_in_old_text", content: "ABC\n", edits: [E("ＡＢＣ", "X")] },
	{ name: "nfkc_fullwidth_with_ws", content: "ＡＢＣ   \n", edits: [E("ABC\nx", "X\ny")] },
	// —— 多编辑组合 ——
	{
		name: "mixed_exact_and_fuzzy",
		content: "alpha   \nbeta\ngamma\n",
		edits: [E("alpha\nbeta", "A\nB"), E("gamma", "G")],
	},
	{
		name: "three_edits_adjacent_lines",
		content: "1\n2\n3\n4\n",
		edits: [E("1", "a"), E("2", "b"), E("3", "c")],
	},
	{
		name: "overlapping_edits",
		content: "abcdef\n",
		edits: [E("abc", "X"), E("cde", "Y")],
	},
	{
		name: "duplicate_second_edit",
		content: "same\nother\nsame\n",
		edits: [E("other", "OTHER"), E("same", "X")],
	},
	{ name: "empty_new_text", content: "a\nb\nc\n", edits: [E("b\n", "")] },
	{ name: "replace_with_whitespace_only", content: "a\nb\n", edits: [E("b", "   ")] },
	{ name: "tab_indentation_fuzzy", content: "\ta\n\tb\n", edits: [E("    a\n    b", "A\nB")] },
];

const results = [];
const dir = await mkdtemp(path.join(tmpdir(), "edit-oracle-"));
try {
	for (const testCase of CASES) {
		const file = path.join(dir, "target.txt");
		await writeFile(file, testCase.content, "utf8");
		const tool = createEditToolDefinition(dir);
		let record = { name: testCase.name, content: testCase.content, edits: testCase.edits };
		try {
			const out = await tool.execute(
				"call_oracle",
				{ path: "target.txt", edits: testCase.edits },
				undefined,
				undefined,
				{ cwd: dir },
			);
			const text = (out.content ?? [])
				.filter((block) => block.type === "text")
				.map((block) => block.text)
				.join("");
			record = {
				...record,
				isError: out.isError === true,
				text,
				diff: out.details?.diff ?? null,
				patch: out.details?.patch ?? null,
				fileAfter: await readFile(file, "utf8"),
			};
		} catch (error) {
			record = { ...record, isError: true, text: `THREW: ${error?.message ?? error}`, fileAfter: null };
		}
		results.push(record);
	}
} finally {
	await rm(dir, { recursive: true, force: true });
}

const outPath = path.join(
	new URL("../..", import.meta.url).pathname,
	"fixtures/edit/cases.json",
);
await import("node:fs/promises").then((fs) =>
	fs.mkdir(path.dirname(outPath), { recursive: true }).then(() =>
		fs.writeFile(outPath, `${JSON.stringify(results, null, 2)}\n`, "utf8"),
	),
);
console.log(`edit-oracle: ${results.length} 组用例 → ${outPath}`);
