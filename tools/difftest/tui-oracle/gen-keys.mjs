// P7-6q/W2：**输入 oracle 夹具**（独立文件 `fixtures-input.json`；既有夹具与 sha256 不动）。
// 用 ref 的真实 `StdinBuffer` 与 `keys.ts` 的解析函数，喂入**字节分片序列**，记录产出的事件序列。
// 超时：`StdinBuffer` 用 `setTimeout`；为**确定性**，本脚本对"孤立 ESC"等用例在 `process()` 后**手动调用 `flush()`**
// 模拟超时触发（`flush` 是私有方法，运行时为普通方法），并记录 `timeoutEmulated: true`。
import fs from "node:fs";
import path from "node:path";

const REF = process.env.PI_TUI_REF_DIR ?? new URL("../../../ref/pi-0.87.1/packages/tui/src/", import.meta.url).pathname;
const { StdinBuffer } = await import(path.join(REF, "stdin-buffer.ts"));
const keys = await import(path.join(REF, "keys.ts"));

/** 固定种子 PRNG（mulberry32）。 */
function mulberry32(seed) {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) >>> 0;
		let t = a;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

/** 跑一个用例：按分片喂入，收集 `data`/`paste` 事件与解析结果。 */
function runCase(id, chunks, { kittyActive = false, emulateTimeout = false } = {}) {
	keys.setKittyProtocolActive(kittyActive);
	const buf = new StdinBuffer();
	const events = [];
	buf.on("data", (seq) => {
		events.push({
			kind: "data",
			seq,
			key: keys.parseKey(seq) ?? null,
			release: keys.isKeyRelease(seq),
			repeat: keys.isKeyRepeat(seq),
		});
	});
	buf.on("paste", (content) => events.push({ kind: "paste", content }));
	for (const chunk of chunks) buf.process(Buffer.from(chunk, "utf8"));
	if (emulateTimeout) {
		// 手动触发超时路径（等价于 setTimeout 到期）
		const flushed = buf.flush?.() ?? [];
		for (const seq of flushed) {
			events.push({ kind: "data", seq, key: keys.parseKey(seq) ?? null, release: keys.isKeyRelease(seq), repeat: keys.isKeyRepeat(seq) });
		}
	}
	buf.dispose?.();
	return { id, kittyActive, timeoutEmulated: emulateTimeout, chunks: chunks.map((c) => Buffer.from(c, "utf8").toString("base64")), events };
}

const cases = [];
const add = (id, chunks, opts) => cases.push(runCase(id, chunks, opts));

// (i) 手写用例
add("h01-printable", ["a", "b", "c"]);
add("h02-utf8-multibyte", ["中", "文"]);
add("h03-ctrl", ["\x01", "\x02", "\x03"]);
add("h04-arrows-legacy", ["\x1b[A", "\x1b[B", "\x1b[C", "\x1b[D"]);
add("h05-home-end-variants", ["\x1b[H", "\x1b[F", "\x1b[1~", "\x1b[4~", "\x1bOH", "\x1bOF"]);
add("h06-insert-delete-pgup-pgdn", ["\x1b[2~", "\x1b[3~", "\x1b[5~", "\x1b[6~"]);
add("h07-fkeys", ["\x1bOP", "\x1b[15~", "\x1b[24~"]);
add("h08-alt", ["\x1ba", "\x1bb"]);
add("h09-shift-ctrl", ["\x1b[1;2A", "\x1b[1;5C"]);
add("h10-kitty-csi-u", ["\x1b[97u", "\x1b[97;5u", "\x1b[97;5:3u"], { kittyActive: true });
add("h11-kitty-alt-key", ["\x1b[97:65;2u"], { kittyActive: true });
add("h12-modify-other-keys", ["\x1b[27;5;97~"], { kittyActive: false });
add("h13-paste-short", ["\x1b[200~hello\x1b[201~"]);
add("h14-paste-newlines", ["\x1b[200~a\nb\nc\x1b[201~"]);
add("h15-paste-split", ["\x1b[200~ab", "cd\x1b[201~"]);
add("h16-paste-long", ["\x1b[200~" + "x".repeat(500) + "\x1b[201~"]);
add("h17-truncated", ["\x1b[1;5"]);
add("h18-lone-esc", ["\x1b"], { emulateTimeout: true });
add("h19-split-csi", ["\x1b", "[", "A"]);
add("h20-kitty-release", ["\x1b[97;1:3u"], { kittyActive: true });

// (i-b) P7-6r0/W5 补强用例（逐类覆盖切分状态机边界）
// A. 孤立 ESC：flush 前（留在缓冲，不产生事件）与 flush 后（产生 escape）
add("s01-esc-before-flush", ["\x1b"]);
add("s02-esc-after-flush", ["\x1b"], { emulateTimeout: true });
add("s03-esc-then-printable-in-timeout", ["\x1b", "a"]);
add("s04-esc-flush-then-printable", ["\x1b"], { emulateTimeout: true });
add("s05-esc-then-csi-in-timeout", ["\x1b", "[A"]);
add("s06-esc-flush-then-csi", ["\x1b"], { emulateTimeout: true });
// B. CSI 逐字节边界分片
add("b01-csi-split-1", ["\x1b", "[", "1", ";", "5", "A"]);
add("b02-csi-split-2", ["\x1b[", "1;5", "A"]);
add("b03-csi-split-3", ["\x1b[1;5", "A"]);
// C. SS3 逐字节边界分片
add("b04-ss3-split-1", ["\x1b", "O", "P"]);
add("b05-ss3-split-2", ["\x1bO", "P"]);
add("b06-ss3-split-3", ["\x1b", "OP"]);
// D. OSC 逐字节边界分片（BEL 与 ST 两种终止）
add("b07-osc-split-bel", ["\x1b", "]", "0", ";", "t", "\x07"]);
add("b08-osc-split-st", ["\x1b]", "0;t", "\x1b", "\\"]);
add("b09-osc-split-mixed", ["\x1b", "]0", ";t\x07"]);
// E. DCS 分片（需 ESC \ 结尾）
add("b10-dcs-split-1", ["\x1b", "P", ">|", "x", "\x1b", "\\"]);
add("b11-dcs-split-2", ["\x1bP", ">|x", "\x1b\\"]);
add("b12-dcs-split-3", ["\x1b", "P>|x\x1b", "\\"]);
// F. APC 分片
add("b13-apc-split-1", ["\x1b", "_", "G", "x", "\x1b", "\\"]);
add("b14-apc-split-2", ["\x1b_", "Gx", "\x1b\\"]);
add("b15-apc-split-3", ["\x1b", "_Gx\x1b", "\\"]);
// G. 老式鼠标 ESC[M 不足 6 字节
add("b16-mouse-short-1", ["\x1b[M"]);
add("b17-mouse-short-2", ["\x1b[", "M", " ", " "]);
add("b18-mouse-short-3", ["\x1b[M", " ", " "]);
add("b19-mouse-full-6", ["\x1b[M", " ", "!", "!"]);
// H. 粘贴开始/结束标记被分片切开
add("p01-paste-start-split", ["\x1b[20", "0~hi\x1b[201~"]);
add("p02-paste-end-split", ["\x1b[200~hi\x1b[20", "1~"]);
add("p03-paste-both-split", ["\x1b[2", "00~ab\x1b[2", "01~"]);
add("p04-paste-start-bytewise", ["\x1b", "[", "2", "0", "0", "~x\x1b[201~"]);
// I. "\x1b\x1b" + [ ] O P _ 的单 ESC 特例（ref `extractCompleteSequences` 特例）
add("e01-esc-esc-csi", ["\x1b\x1b[27;1:3u"]);
add("e02-esc-esc-osc", ["\x1b\x1b]0;t\x07"]);
add("e03-esc-esc-ss3", ["\x1b\x1bOP"]);
add("e04-esc-esc-dcs", ["\x1b\x1bP>|x\x1b\\"]);
add("e05-esc-esc-apc", ["\x1b\x1b_Gx\x1b\\"]);
add("e06-esc-esc-plain", ["\x1b\x1ba"]);

// (ii) 随机序列（≥200）
const fragments = ["a", "Z", "1", "中", "\x01", "\x03", "\x1b[A", "\x1b[B", "\x1b[3~", "\x1bOP", "\x1ba", "\x1b[1;5A", "\x1b[97;5u", "\x1b[27;5;97~", "\x1b[200~p\x1b[201~"];
for (let n = 0; n < 200; n += 1) {
	const seed = 9000 + n;
	const rand = mulberry32(seed);
	const parts = 1 + Math.floor(rand() * 4);
	const chunks = [];
	for (let k = 0; k < parts; k += 1) chunks.push(fragments[Math.floor(rand() * fragments.length)]);
	const kittyActive = n % 2 === 1;
	cases.push({ ...runCase(`r${String(n).padStart(3, "0")}`, chunks, { kittyActive }), seed });
}

const out = process.argv[2] ?? new URL("./fixtures-input.json", import.meta.url).pathname;
fs.writeFileSync(out, JSON.stringify({ generator: "tools/difftest/tui-oracle/gen-keys.mjs", tsVersion: "0.87.1", cases }));
// 供 Rust 单测读取的**无依赖文本侧车**（同一份数据；JSON 仍为正式夹具）：
//   case <id> <kittyActive 0|1> <timeoutEmulated 0|1> <分片数>
//   chunk <base64>
//   event data <base64(seq)> <key|-> <release 0|1> <repeat 0|1>
//   event paste <base64(content)>
const sidecarPath = out.replace(/\.json$/, ".lines.txt");
const b64 = (s) => Buffer.from(s, "utf8").toString("base64");
const sc = [];
for (const c of cases) {
	sc.push(`case ${c.id} ${c.kittyActive ? 1 : 0} ${c.timeoutEmulated ? 1 : 0} ${c.chunks.length}`);
	for (const ch of c.chunks) sc.push(`chunk ${ch}`);
	for (const e of c.events) {
		if (e.kind === "paste") sc.push(`event paste ${b64(e.content)}`);
		else sc.push(`event data ${b64(e.seq)} ${e.key ?? "-"} ${e.release ? 1 : 0} ${e.repeat ? 1 : 0}`);
	}
}
fs.writeFileSync(sidecarPath, `${sc.join("\n")}\n`);
console.log(`输入夹具用例=${cases.length}（手写 20 + W5 补强 35 + 随机 200） 输出=${out} 大小=${(fs.statSync(out).size / 1024).toFixed(1)} KB 侧车=${(fs.statSync(sidecarPath).size / 1024).toFixed(1)} KB`);
