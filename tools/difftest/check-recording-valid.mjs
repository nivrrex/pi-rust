#!/usr/bin/env node
// P7-6v/W1：**录制有效性闸门**。
//
// 用途（仅此一项）：判定"这次录制是否值得拿去做 same/different 判定"。
// **不参与** same/different 判定，也不改动任何既有判定规则。
//
// 背景：#16 曾把"mock 不可达 ⇒ agent 无回复"误判成 pi-rs 渲染缺陷；本闸门把该类无效录制拦在比对之前。
//
// 用法：
//   node tools/difftest/check-recording-valid.mjs --dir <录制产出目录, 含 run-1/> [--golden <golden 目录>]
// 退出码：0=有效；1=无效（原因打在 stdout）；2=用法错误。
//
// 规则：
//   a. **需要 provider 请求的录制**（判定依据：golden 目录里存在 `requests.jsonl`，或调用方给 `--expect-request`）
//      ⇒ 本项目录制的 `requests.jsonl` 必须存在且行数 ≥ 1；否则无效。
//      说明：`golden-tui/*/run-1/` 不含 requests.jsonl（当年未留存），因此默认对**所有** TUI 场景都要求
//      请求日志存在——TUI 场景全部声明了 `responses`（`scenarios-tui/*.json` 里均有该字段），必然要向
//      mock 发请求；缺失即说明 mock 未被访问（典型原因：netns 内没有 `ip link set lo up`）。
//   b. **assistant 文本在场**：若给出 `--golden` 且 golden 的 `snapshot.txt` 里存在"非 UI 文本行"，
//      则本录制的 raw（去 ANSI）必须包含 golden 的**最后一条非 UI 文本行**；否则无效。
//      "非 UI" 判定：不以 `─`/`│`/`┌`/`└`/`├`/`[Context]`/`↑`/数字+`%` 开头，不是绝对路径，不是 `say hi` 之类
//      的场景 prompt（prompt 从场景 JSON 读不到时退化为只排除上述形态）。
//   c. 无 `FAILED.md`；`run-1/raw.bin` 存在且非空。
import fs from "node:fs";
import path from "node:path";

function parseArgs(argv) {
	const get = (n, d) => { const i = argv.indexOf(n); return i >= 0 ? argv[i + 1] : d; };
	return { dir: get("--dir"), golden: get("--golden"), scenario: get("--scenario"), expectRequest: argv.includes("--expect-request") };
}

const ANSI = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g;
const strip = (text) => text.replace(ANSI, "");

const UI_LINE = /^\s*(?:[─│┌┐└┘├┤┬┴┼]|[↑↓]\d|\d+\.\d+%|[●○]|─)/;
const isUi = (line) => {
	const t = line.trim();
	if (t === "") return true;
	if (UI_LINE.test(t)) return true;
	if (t.startsWith("/")) return true; // 绝对路径（cwd 行）
	if (t.includes("<TMP>")) return true; // golden snapshot 里的路径占位符
	if (/^\d+(\.\d+)?%/.test(t)) return true;
	if (/^\[Context\]$/.test(t)) return true;
	if (/^\d{4}-\d{2}-\d{2}T/.test(t)) return true;
	return false;
};

function finalScreenLines(rawPath) {
	const text = strip(fs.readFileSync(rawPath, "utf8"));
	// 取最后一个同步区之后的可见文本：直接按行切并保留非空行（足够做"文本在场"判断）
	return text.split(/[\r\n]+/).map((l) => l.replace(/\s+$/, ""));
}

function main() {
	const { dir, golden, scenario, expectRequest } = parseArgs(process.argv.slice(2));
	// 是否"需要 provider 请求"：由**场景 JSON 的 `responses` 字段**判定（`scenarios-tui/*.json` 全部有该字段，
	// 必然要向 mock 发请求）；golden 目录不含 requests.jsonl（当年未留存），故不能作为依据。
	let scenarioNeedsRequest = expectRequest;
	if (scenario && fs.existsSync(scenario)) {
		try {
			const s = JSON.parse(fs.readFileSync(scenario, "utf8"));
			// 有 `responses` **且**场景真的会提交用户 prompt（存在 `type` 步骤且其 text 含 `prompt`），
			// 才需要 provider 请求。纯本地场景（如 T02 `/quit` 退出）虽有 responses，但从不发请求。
			const typed = (s.tui?.steps ?? []).filter((st) => st.action === "type").map((st) => String(st.text ?? ""));
			const submitsPrompt = typeof s.prompt === "string" && s.prompt.length > 0
				&& typed.some((text) => text.includes(s.prompt));
			scenarioNeedsRequest = Array.isArray(s.responses) && s.responses.length > 0 && submitsPrompt;
		} catch { /* 场景解析失败时保持 expectRequest */ }
	}
	const isGoldenDir = Boolean(golden) && path.resolve(golden) === path.resolve(dir);
	if (!dir) { console.error("用法: node check-recording-valid.mjs --dir <录制产出目录> [--golden <golden 目录>]"); process.exit(2); }
	// 兼容两种入参：父目录（含 run-1/）或 run-1 目录本身
	const run1 = fs.existsSync(path.join(dir, "raw.bin")) ? dir : path.join(dir, "run-1");
	const rawPath = path.join(run1, "raw.bin");
	const reasons = [];

	// c. FAILED.md / raw 非空
	if (fs.existsSync(path.join(dir, "FAILED.md"))) reasons.push("存在 FAILED.md");
	if (!fs.existsSync(rawPath)) reasons.push("缺少 run-1/raw.bin");
	else if (fs.statSync(rawPath).size === 0) reasons.push("run-1/raw.bin 为空");

	// a. provider 请求日志
	const reqCandidates = ["requests.jsonl", "../requests.jsonl"];
	const reqPath = reqCandidates.map((p) => path.join(run1, p)).find((p) => fs.existsSync(p))
		?? path.join(dir, "requests.jsonl");
	const needsRequest = !isGoldenDir && scenarioNeedsRequest;
	if (needsRequest) {
		if (!fs.existsSync(reqPath)) reasons.push(`缺少 requests.jsonl（mock 未被访问，常见原因：netns 内未 ip link set lo up）`);
		else {
			const lines = fs.readFileSync(reqPath, "utf8").split("\n").filter(Boolean).length;
			if (lines < 1) reasons.push("requests.jsonl 行数为 0（mock 未被访问）");
		}
	}

	// b. assistant 文本在场
	if (golden) {
		const snap = path.join(golden, "snapshot.txt");
		const goldenRaw = path.join(golden, "raw.bin");
		let goldenLines = [];
		if (fs.existsSync(snap)) goldenLines = fs.readFileSync(snap, "utf8").split("\n");
		else if (fs.existsSync(goldenRaw)) goldenLines = finalScreenLines(goldenRaw);
		const assistantish = goldenLines.map((l) => l.trim()).filter((l) => l !== "" && !isUi(l));
		if (assistantish.length > 0 && fs.existsSync(rawPath)) {
			const mine = finalScreenLines(rawPath).join("\n");
			const last = assistantish[assistantish.length - 1];
			if (!mine.includes(last)) reasons.push(`raw 中缺少 golden 的最后一条非 UI 文本行：${JSON.stringify(last.slice(0, 60))}`);
		}
	}

	if (reasons.length > 0) {
		console.log(`[recording-invalid] ${dir}`);
		for (const r of reasons) console.log(`  - ${r}`);
		process.exit(1);
	}
	console.log(`[recording-valid] ${dir}`);
	process.exit(0);
}

main();
