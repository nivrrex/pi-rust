#!/usr/bin/env node
// F53d/D：`renderIntraLineDiff` 的 oracle。
//
// 从 TS 包依赖的 jsdiff（`diff@8.0.4`，`ref/pi-0.87.1/packages/coding-agent/package.json:59`）
// 直接调用 `diffWords`，并按 `ref/pi-0.87.1/packages/coding-agent/src/modes/interactive/
// components/diff.ts:33-77` 的 `renderIntraLineDiff` 逐行复刻（去样式，只保留文本），
// 产出 `期望 removed` / `期望 added` 两行，供 Rust 实现逐字对拍。
//
// 只用于测试，不进入产品。
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..", "..");
const require = createRequire(join(repo, "tools", "difftest", "jsdiff-oracle", "package.json"));
const Diff = require(process.env.JSDIFF_PATH || "/root/node_modules/.pnpm/diff@8.0.4/node_modules/diff");

// ref diff.ts:33-77
function renderIntraLineDiff(oldContent, newContent) {
	const wordDiff = Diff.diffWords(oldContent, newContent);
	let removedLine = "";
	let addedLine = "";
	let isFirstRemoved = true;
	let isFirstAdded = true;
	for (const part of wordDiff) {
		if (part.removed) {
			let value = part.value;
			if (isFirstRemoved) {
				const leadingWs = value.match(/^(\s*)/)?.[1] || "";
				value = value.slice(leadingWs.length);
				removedLine += leadingWs;
				isFirstRemoved = false;
			}
			if (value) {
				removedLine += value;
			}
		} else if (part.added) {
			let value = part.value;
			if (isFirstAdded) {
				const leadingWs = value.match(/^(\s*)/)?.[1] || "";
				value = value.slice(leadingWs.length);
				addedLine += leadingWs;
				isFirstAdded = false;
			}
			if (value) {
				addedLine += value;
			}
		} else {
			removedLine += part.value;
			addedLine += part.value;
		}
	}
	return { removed: removedLine, added: addedLine };
}

const cases = JSON.parse(readFileSync(join(here, "cases.json"), "utf8"));
const out = cases.map((c) => {
	const r = renderIntraLineDiff(c.old, c.new);
	return { name: c.name, removed: r.removed, added: r.added };
});
process.stdout.write(`${JSON.stringify(out, null, "\t")}\n`);
