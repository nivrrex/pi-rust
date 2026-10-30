// diffLines / createTwoFilesPatch 的 oracle：调**真实的 jsdiff**，把每组输入与期望落成 JSON，
// 供 Rust 侧逐项比对。
//
// 为什么要有它：`edit` 的 `details` 里 `diff`/`patch` 由 jsdiff 产出（`edit-diff.ts:365-380`），
// 而 jsdiff 有多处**未文档化行为**（change 切分、空白去重、前缀/后缀裁剪、patch 头格式）。
// 这些只能靠对照真实实现来定，不能凭样本猜。
//
// 参照实现的位置（2026-xx 实测）：
//   <diff 包目录>（用 DIFF_PACKAGE 指定）                    （项目树外；`pi-ts-0.87.1/node_modules` 下没有）
// 也可用 `DIFF_PACKAGE` 指到别处的同名包。
//
// 另：`edit-diff.ts:376-380` 的 `generateDiffString`（带行号与上下文）**不在本 oracle 范围**，
// 它是在 `diffLines` 之上自己拼格式，等卡片 2 再做。

import { createRequire } from "node:module";
import { writeFile, mkdir } from "node:fs/promises";
import path from "node:path";

const require = createRequire(import.meta.url);
const packagePath = process.env.DIFF_PACKAGE;
if (!packagePath) {
  console.error("diff-oracle: 需要 DIFF_PACKAGE 指向 `diff` 包目录（不再硬编码本机路径）");
  process.exit(2);
}
const Diff = require(packagePath);

/** `[名字, 旧文本, 新文本]`。覆盖空行、末行无换行、CRLF、重复行、整段替换等。 */
const LINES_CASES = [
  ["identical", "a\nb\n", "a\nb\n"],
  ["single_line_change", "a\nb\nc\n", "a\nB\nc\n"],
  ["insert_in_middle", "a\nc\n", "a\nb\nc\n"],
  ["delete_in_middle", "a\nb\nc\n", "a\nc\n"],
  ["whole_block_replace", "a\nb\nc\n", "x\ny\nz\n"],
  ["empty_to_text", "", "a\n"],
  ["text_to_empty", "a\n", ""],
  ["both_empty", "", ""],
  ["blank_line_inserted", "a\nb\n", "a\n\nb\n"],
  ["blank_line_removed", "a\n\nb\n", "a\nb\n"],
  ["only_blank_lines", "\n\n\n", "\n"],
  ["no_trailing_newline_change", "a\nb", "a\nB"],
  ["trailing_newline_added", "a\nb", "a\nb\n"],
  ["trailing_newline_removed", "a\nb\n", "a\nb"],
  ["crlf_same", "a\r\nb\r\n", "a\r\nb\r\n"],
  ["crlf_change", "a\r\nb\r\n", "a\r\nB\r\n"],
  ["crlf_vs_lf", "a\r\nb\r\n", "a\nb\n"],
  ["duplicate_lines_keep", "x\nx\nx\n", "x\nx\nx\n"],
  ["duplicate_lines_change_one", "x\nx\nx\n", "x\ny\nx\n"],
  ["duplicate_lines_delete_one", "x\nx\nx\n", "x\nx\n"],
  ["swap_adjacent", "a\nb\n", "b\na\n"],
  ["prepend", "b\nc\n", "a\nb\nc\n"],
  ["append", "a\nb\n", "a\nb\nc\n"],
  ["unicode", "中文\n第二行\n", "中文\n第三行\n"],
  ["long_line", `${"x".repeat(200)}\n`, `${"y".repeat(200)}\n`],
  ["many_small_changes", "1\n2\n3\n4\n5\n6\n", "1\nX\n3\nY\n5\n6\n"],
  ["first_line_change", "a\nb\nc\n", "A\nb\nc\n"],
  ["last_line_change", "a\nb\nc\n", "a\nb\nC\n"],
  ["repeated_block_removed", "a\nb\na\nb\n", "a\nb\n"],
  ["whitespace_only_change", "a\n \n", "a\n  \n"],
  ["tab_vs_space", "a\tb\n", "a b\n"],
  ["empty_string_vs_newline", "", "\n"],
  ["newline_vs_empty_string", "\n", ""],
];

/** 给 `createTwoFilesPatch` 的用例（context 固定 4，与 `edit-diff.ts:368` 一致）。 */
const PATCH_CASES = [
  ["single_line", "a\nb\nc\n", "a\nB\nc\n"],
  ["insert_in_middle", "a\nc\n", "a\nb\nc\n"],
  ["no_trailing_newline", "a\nb", "a\nB"],
  ["crlf", "a\r\nb\r\n", "a\r\nB\r\n"],
  ["whole_block_replace", "a\nb\nc\n", "x\ny\nz\n"],
  // 卡片 2 补充：上下文边界与 hunk 切分是 formatPatch/structuredPatch 的边角行为所在。
  ["addition_only_at_end", "a\n", "a\nb\n"],
  ["deletion_only_at_end", "a\nb\n", "a\n"],
  ["two_hunks_far_apart", `${Array.from({ length: 20 }, (_, i) => `line${i}\n`).join("")}`, `${Array.from({ length: 20 }, (_, i) => (i === 1 ? "X1\n" : i === 18 ? "X18\n" : `line${i}\n`)).join("")}`],
  ["two_changes_within_context", "a\nb\nc\nd\ne\nf\ng\n", "a\nB\nc\nd\ne\nF\ng\n"],
  ["empty_to_text", "", "a\n"],
  ["text_to_empty", "a\n", ""],
  ["only_change_first_line", "a\nb\nc\n", "A\nb\nc\n"],
  ["only_change_last_line", "a\nb\nc\n", "a\nb\nC\n"],
  ["blank_line_change", "a\n\nb\n", "a\nX\nb\n"],
];

const records = [];

for (const [name, oldText, newText] of LINES_CASES) {
  records.push({
    name,
    kind: "diffLines",
    oldText,
    newText,
    // jsdiff 的 change 对象：`{value, count, added?, removed?}`
    changes: Diff.diffLines(oldText, newText),
  });
}

for (const [name, oldText, newText] of PATCH_CASES) {
  records.push({
    name,
    kind: "createTwoFilesPatch",
    oldText,
    newText,
    patch: Diff.createTwoFilesPatch("f.txt", "f.txt", oldText, newText, undefined, undefined, {
      context: 4,
      headerOptions: Diff.FILE_HEADERS_ONLY,
    }),
  });
}

const outPath = path.join(new URL("../..", import.meta.url).pathname, "fixtures/diff/cases.json");
await mkdir(path.dirname(outPath), { recursive: true });
await writeFile(
  outPath,
  `${JSON.stringify({ package: packagePath, records }, null, 2)}\n`,
  "utf8",
);
console.log(`diff-oracle: ${records.length} 组（diffLines ${LINES_CASES.length} + patch ${PATCH_CASES.length}）→ ${outPath}`);
