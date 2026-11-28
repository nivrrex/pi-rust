// `generateDiffString`（`edit-diff.ts:376-498`）的 oracle：调 **pi 包的真实实现**。
//
// 为什么不复用 `diff-oracle.mjs`：`generateDiffString` **不属于 jsdiff**，它是
// `edit-diff.ts` 自己的显示格式化（带行号、带 `...` 折叠、并给出 `firstChangedLine`）。
// jsdiff 那边只有 `diffLines`/`patch` 两个路径。
//
// 用法：
//   PI_CODING_AGENT_INDEX=<包>/dist/index.js node tools/oracle/diff-display-oracle.mjs
// 输出：fixtures/diff/display-cases.json

import { writeFile, mkdir } from "node:fs/promises";
import path from "node:path";

const INDEX = process.env.PI_CODING_AGENT_INDEX;
if (!INDEX) {
  console.error("必须设置 PI_CODING_AGENT_INDEX 指向包的 dist/index.js");
  process.exit(2);
}
const { generateDiffString } = await import(INDEX);

/** `[名字, 旧文本, 新文本]`。覆盖 leading/trailing/both 三分支与 `...` 折叠边界。 */
const CASES = [
  ["single_change", "a\nb\nc\n", "a\nB\nc\n"],
  ["change_at_first_line", "a\nb\nc\n", "A\nb\nc\n"],
  ["change_at_last_line", "a\nb\nc\n", "a\nb\nC\n"],
  ["insert_line", "a\nb\n", "a\nx\nb\n"],
  ["delete_line", "a\nb\nc\n", "a\nc\n"],
  ["multi_line_change", "a\nb\nc\nd\n", "a\nX\nY\nd\n"],
  // 两处改动、间隔 <= context*2 → both 分支的"全显"路径。
  ["two_changes_short_gap", "a\nb\nc\nd\ne\nf\ng\n", "a\nB\nc\nd\ne\nF\ng\n"],
  // 两处改动、间隔 > context*2 → both 分支的"折叠"路径（应出现 `...`）。
  [
    "two_changes_long_gap",
    `${Array.from({ length: 20 }, (_, i) => `l${i}\n`).join("")}`,
    `${Array.from({ length: 20 }, (_, i) => (i === 1 ? "X1\n" : i === 18 ? "X18\n" : `l${i}\n`)).join("")}`,
  ],
  // 只有前导改动 → leading 分支。
  [
    "leading_change_only",
    `${Array.from({ length: 15 }, (_, i) => `l${i}\n`).join("")}`,
    `X0\n${Array.from({ length: 14 }, (_, i) => `l${i + 1}\n`).join("")}`,
  ],
  // 只有尾随改动 → trailing 分支。
  [
    "trailing_change_only",
    `${Array.from({ length: 15 }, (_, i) => `l${i}\n`).join("")}`,
    `${Array.from({ length: 14 }, (_, i) => `l${i}\n`).join("")}X14\n`,
  ],
  ["no_change", "a\nb\n", "a\nb\n"],
  ["empty_to_text", "", "a\n"],
  ["text_to_empty", "a\n", ""],
  ["no_trailing_newline", "a\nb", "a\nB"],
  ["crlf", "a\r\nb\r\n", "a\r\nB\r\n"],
  ["gap_exactly_context_times_two", "a\nb\nc\nd\ne\nf\n", "a\nB\nc\nd\ne\nf\n"],
  ["gap_one_over_context_times_two", "a\nb\nc\nd\ne\nf\ng\n", "a\nB\nc\nd\ne\nf\ng\n"],
];

const records = [];
for (const [name, oldText, newText] of CASES) {
  const result = generateDiffString(oldText, newText);
  records.push({
    name,
    oldText,
    newText,
    diff: result.diff,
    firstChangedLine: result.firstChangedLine ?? null,
  });
}

const outPath = path.join(new URL("../..", import.meta.url).pathname, "fixtures/diff/display-cases.json");
await mkdir(path.dirname(outPath), { recursive: true });
await writeFile(outPath, `${JSON.stringify({ records }, null, 2)}\n`, "utf8");
console.log(`diff-display-oracle: ${records.length} 组 → ${outPath}`);
