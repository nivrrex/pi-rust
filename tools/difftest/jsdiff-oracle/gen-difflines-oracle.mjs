#!/usr/bin/env node
// F53d/D：`renderDiff`（ref `components/diff.ts:79-160`）的文本 oracle（去颜色）。
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..", "..");
const require = createRequire(join(here, "package.json"));
const Diff = require(process.env.JSDIFF_PATH || "/root/node_modules/.pnpm/diff@8.0.4/node_modules/diff");

const strip = (s) => s.replace(/\x1b\[[0-9;]*m/g, "");
// 无样式：`theme.fg(name, text)` 返回 `text`（第一参是颜色名）。
const theme = new Proxy({}, { get: () => (_name, text) => text });

function parseDiffLine(line) {
	const match = line.match(/^([+-\s])(\s*\d*)\s(.*)$/);
	if (!match) return null;
	return { prefix: match[1], lineNum: match[2], content: match[3] };
}
const replaceTabs = (t) => t.replace(/\t/g, "   ");

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
			if (value) removedLine += value;
		} else if (part.added) {
			let value = part.value;
			if (isFirstAdded) {
				const leadingWs = value.match(/^(\s*)/)?.[1] || "";
				value = value.slice(leadingWs.length);
				addedLine += leadingWs;
				isFirstAdded = false;
			}
			if (value) addedLine += value;
		} else {
			removedLine += part.value;
			addedLine += part.value;
		}
	}
	return { removedLine, addedLine };
}

function renderDiff(diffText) {
	const lines = diffText.split("\n");
	const result = [];
	let i = 0;
	while (i < lines.length) {
		const line = lines[i];
		const parsed = parseDiffLine(line);
		if (!parsed) { result.push(strip(theme.fg("toolDiffContext", line))); i++; continue; }
		if (parsed.prefix === "-") {
			const removedLines = [];
			while (i < lines.length) {
				const p = parseDiffLine(lines[i]);
				if (!p || p.prefix !== "-") break;
				removedLines.push({ lineNum: p.lineNum, content: p.content });
				i++;
			}
			const addedLines = [];
			while (i < lines.length) {
				const p = parseDiffLine(lines[i]);
				if (!p || p.prefix !== "+") break;
				addedLines.push({ lineNum: p.lineNum, content: p.content });
				i++;
			}
			if (removedLines.length === 1 && addedLines.length === 1) {
				const removed = removedLines[0];
				const added = addedLines[0];
				const { removedLine, addedLine } = renderIntraLineDiff(
					replaceTabs(removed.content), replaceTabs(added.content));
				result.push(strip(theme.fg("toolDiffRemoved", `-${removed.lineNum} ${removedLine}`)));
				result.push(strip(theme.fg("toolDiffAdded", `+${added.lineNum} ${addedLine}`)));
			} else {
				for (const r of removedLines) result.push(`-${r.lineNum} ${replaceTabs(r.content)}`);
				for (const a of addedLines) result.push(`+${a.lineNum} ${replaceTabs(a.content)}`);
			}
		} else if (parsed.prefix === "+") {
			result.push(`+${parsed.lineNum} ${replaceTabs(parsed.content)}`);
			i++;
		} else {
			result.push(` ${parsed.lineNum} ${replaceTabs(parsed.content)}`);
			i++;
		}
	}
	return result;
}

const cases = JSON.parse(readFileSync(join(here, "diff-cases.json"), "utf8"));
const out = cases.map((c) => ({ name: c.name, lines: renderDiff(c.diff) }));
process.stdout.write(`${JSON.stringify(out, null, "\t")}\n`);
