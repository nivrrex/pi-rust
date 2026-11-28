// P7-6m/W0(d)：统计 golden raw 中**渲染内光标定位**序列（只读诊断；不改 analyze-raw 既有列语义）。
//
// 每个 render 以 `\e[?2026h` 开始、`\e[?2026l` 结束；render 结束后到下一个 `h` 之前写出的
// `\e[{n}G` / `\e[{n}A` / `\e[{n}B` / `\e[?25h` / `\e[?25l` 属 positionHardwareCursor 的输出。
import fs from "node:fs";
import { renderScreen, snapOffsetToFrameEnd, trimRawToOffset, lastSnapshotOffset } from "./compare-tui.mjs";

const RE_H = /\x1b\[\?2026h/g;
const RE_L = /\x1b\[\?2026l/g;
const RE_G = /\x1b\[(\d+)G/g;
const RE_V = /\x1b\[(\d+)[AB]/g;
const RE_CUR = /\x1b\[\?25([hl])/g;

function positions(re, text) {
	const out = [];
	let m;
	re.lastIndex = 0;
	while ((m = re.exec(text)) !== null) out.push({ index: m.index, value: m[1] });
	return out;
}

export function analyzeCursor(raw) {
	const text = raw.toString("latin1");
	const ends = positions(RE_L, text);
	const begins = positions(RE_H, text);
	const gs = positions(RE_G, text);
	const vs = positions(RE_V, text);
	const cursors = positions(RE_CUR, text);
	const renders = [];
	for (let i = 0; i < ends.length; i += 1) {
		const start = ends[i].index;
		const next = i + 1 < begins.length ? begins[i + 1].index : start;
		const stop = next > start ? next : text.length;
		const inWindow = (p) => p.index > start && p.index < stop;
		renders.push({
			renderIndex: i,
			gCount: gs.filter(inWindow).length,
			gCols: gs.filter(inWindow).map((p) => Number.parseInt(p.value, 10)),
			moveCount: vs.filter(inWindow).length,
			cursorOps: cursors.filter(inWindow).map((p) => p.value),
		});
	}
	return { totalRenders: ends.length, gTotal: gs.length, renders };
}

const args = process.argv.slice(2);
if (args.length === 0) {
	console.error("用法: node tools/difftest/analyze-cursor.mjs <id>...");
	process.exit(2);
}
for (const id of args) {
	const dir = `golden-tui/${id}/run-1`;
	const raw = fs.readFileSync(`${dir}/raw.bin`);
	const analysis = analyzeCursor(raw);
	// 编辑器行的可见宽度：取末帧最后 3 行中"非边框、非 cwd、非状态行"的那一行（近似），仅作对照参考。
	const trimmed = trimRawToOffset(raw, snapOffsetToFrameEnd(raw, lastSnapshotOffset(`${dir}/meta.json`)));
	const screen = renderScreen(trimmed, 100, 30).map((l) => l.replace(/\s+$/, ""));
	const editorLike = screen.filter((l) => l !== "" && !/^─+$/.test(l) && !/^\/tmp\//.test(l) && !/%\/\d+k/.test(l)).at(-1) ?? "";
	console.log(`# ${id} render 次数=${analysis.totalRenders} \e[nG 总数=${analysis.gTotal}`);
	analysis.renders.forEach((r) => {
		console.log(
			`  render ${r.renderIndex}: G 次数=${r.gCount} cols=${JSON.stringify(r.gCols)} 位移(A/B)=${r.moveCount} 光标=${JSON.stringify(r.cursorOps)}`,
		);
	});
	console.log(`  参考：末帧"编辑器样"行=${JSON.stringify(editorLike.slice(0, 40))}（宽=${editorLike.length}）`);
}
