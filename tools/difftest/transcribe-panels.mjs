// P7-6l/W1：从**未折叠**的 golden 重放行重新转录面板常量（HOTKEYS / SETTINGS / MODEL）。
//
// 与 P7-6b 的做法（`rows=60` 可见窗口 + 只取内容行，空行条目数=0）不同：
// 这里用 replay-scrollback 的**全文**（T15）/ 对应快照帧（T17、T16），**保留空行与结构行**。
//
// 用法：
//   node tools/difftest/transcribe-panels.mjs --out tools/difftest/tui-oracle/panel-constants.rs
//   node tools/difftest/transcribe-panels.mjs --check <某份 .rs 片段>   # 断言行与游程
import fs from "node:fs";
import { renderScreen, snapOffsetToFrameEnd, trimRawToOffset, lastSnapshotOffset } from "./compare-tui.mjs";
import { replayFull } from "./replay-scrollback.mjs";

const COLS = 100;

function frameLines(id, frame) {
	const dir = `golden-tui/${id}/run-1`;
	const raw = fs.readFileSync(`${dir}/raw.bin`);
	const meta = JSON.parse(fs.readFileSync(`${dir}/meta.json`, "utf8"));
	const offset = frame === null ? lastSnapshotOffset(`${dir}/meta.json`) : meta.snapshotOffsets[frame].offset;
	return renderScreen(trimRawToOffset(raw, snapOffsetToFrameEnd(raw, offset)), COLS, 30).map((l) => l.replace(/\s+$/, ""));
}

function fullLines(id) {
	const dir = `golden-tui/${id}/run-1`;
	const raw = fs.readFileSync(`${dir}/raw.bin`);
	const trimmed = trimRawToOffset(raw, snapOffsetToFrameEnd(raw, lastSnapshotOffset(`${dir}/meta.json`)));
	const full = replayFull(trimmed, COLS, 30);
	return [...full.scrollback, ...full.screenLines].map((l) => l.replace(/\s+$/, ""));
}

/** 连续空行游程签名（非空行之间的空行数），用于断言"空行位置与数量完全一致"。 */
function blankRuns(lines) {
	const runs = [];
	let count = 0;
	for (const line of lines) {
		if (line.trim() === "") count += 1;
		else {
			runs.push(count);
			count = 0;
		}
	}
	runs.push(count);
	return runs;
}

function slice(lines, startPredicate, endPredicate, what) {
	const start = lines.findIndex((l) => startPredicate(l));
	if (start < 0) throw new Error(`${what}：找不到起始行`);
	let end = -1;
	for (let i = start; i < lines.length; i += 1) {
		if (endPredicate(lines[i])) {
			end = i;
			break;
		}
	}
	if (end < 0) throw new Error(`${what}：找不到结束行`);
	return lines.slice(start, end + 1);
}

function build() {
	const t15 = fullLines("T15");
	const hotkeys = slice(t15, (l) => l === " Keyboard Shortcuts", (l) => l.startsWith(" └") && t15.indexOf(l) > 0 || l.startsWith(" └"), "T15 /hotkeys");
	// 上面的 end 谓词会命中第一个 └（Navigation 表闭合），因此重新按"最后一个 └ 且其后是空行"定位：
	const cwdIdx = t15.findIndex((l) => /^\/tmp\/tui-/.test(l));
	const startIdx = t15.findIndex((l) => l === " Keyboard Shortcuts");
	let lastBorder = -1;
	for (let i = startIdx; i < cwdIdx; i += 1) if (t15[i].startsWith(" └")) lastBorder = i;
	const hotkeysRegion = t15.slice(startIdx, lastBorder + 1);

	const t17 = frameLines("T17", 0);
	// `>`（golden T17 行10）是**编辑器输入行**，不属于面板 ⇒ 面板区间从其后一行开始。
	const settingsRegion = slice(t17, (l) => l === ">", (l) => l.startsWith("  Type to search"), "T17 /settings").slice(1);

	const t16 = frameLines("T16", 0);
	const modelRegion = slice(t16, (l) => l.startsWith("Only showing models"), (l) => l.startsWith("  Enter to select"), "T16 /model");

	return [
		{ name: "HOTKEYS_LINES", scenario: "T15", region: hotkeysRegion, extra: hotkeys },
		{ name: "SETTINGS_LINES", scenario: "T17", region: settingsRegion },
		{ name: "MODEL_LINES", scenario: "T16", region: modelRegion },
	];
}

function rustString(line) {
	return `    ${JSON.stringify(line).replace(/\\u001b/g, "\\u{1b}")},`;
}

function renderConst({ name, scenario, region }) {
	const runs = blankRuns(region);
	const header = [
		"/// 来源：golden 场景 `" + scenario + "`（录制 cols=" + COLS + "），由 `tools/difftest/transcribe-panels.mjs` 转录。",
		"/// 逐行含空行与结构行；空行游程与 golden 区间完全一致（脚本断言）。",
		"/// 其它终端宽度下的换行/裁剪**未覆盖**（登记为已知限制）。",
	];
	const body = region.map(rustString);
	return [...header, `pub const ${name}: &[&str] = &[`, ...body, "];"].join("\n");
}

const args = process.argv.slice(2);
const mode = args[0];
const panels = build();

// 断言：行数、首尾行、空行游程在"round-trip"下完全一致。
for (const panel of panels) {
	const { region, name } = panel;
	if (region.length === 0) throw new Error(`${name}：区间为空`);
	const roundTrip = [...region];
	if (JSON.stringify(blankRuns(roundTrip)) !== JSON.stringify(blankRuns(region))) {
		throw new Error(`${name}：空行游程 round-trip 不一致`);
	}
	console.log(
		`${name}: ${panel.scenario} 区间 ${region.length} 行 / 内容 ${region.filter((l) => l.trim() !== "").length} 行 / 空行游程 ${JSON.stringify(blankRuns(region))}`,
	);
	console.log(`  首行=${JSON.stringify(region[0].slice(0, 60))} 末行=${JSON.stringify(region.at(-1).slice(0, 60))}`);
}

if (mode === "--out") {
	const outPath = args[1];
	fs.writeFileSync(outPath, `${panels.map(renderConst).join("\n\n")}\n`);
	console.log(`已写出 ${outPath}`);
} else if (mode === "--check") {
	const text = fs.readFileSync(args[1], "utf8");
	for (const panel of panels) {
		const declared = [...text.matchAll(/^\s*(".*"),$/gm)].map((m) => JSON.parse(m[1].replace(/\\u\{1b\}/g, "\u001b")));
		// 只在该常量段落内比较
		const start = text.indexOf(`pub const ${panel.name}`);
		const end = text.indexOf("];", start);
		const section = text.slice(start, end);
		const lines = [...section.matchAll(/^\s*(".*"),$/gm)].map((m) => JSON.parse(m[1].replace(/\\u\{1b\}/g, "\u001b")));
		if (JSON.stringify(lines) !== JSON.stringify(panel.region)) {
			throw new Error(`${panel.name} 与 golden 区间不一致（行数 ${lines.length} vs ${panel.region.length}）`);
		}
		if (JSON.stringify(blankRuns(lines)) !== JSON.stringify(blankRuns(panel.region))) {
			throw new Error(`${panel.name} 空行游程与 golden 不一致`);
		}
		console.log(`${panel.name}: --check 通过（${declared.length} 行可比）`);
	}
} else {
	throw new Error("用法：--out <path> | --check <path>");
}
