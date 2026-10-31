// P7-4h/W0(a)：逐帧统计（dev 诊断工具，不改任何判据）。
// 用法：node tools/difftest/diag-tui-frames.mjs <raw.bin> <meta.json> [cols] [rows]
// 输出每帧：字节数、\n 数、\r\n 数、光标上移序列及其 n、帧后渲染屏幕的有效行数（最后一个非空行+1）。
import fs from "fs";
import { renderScreen, normalizeScreen, lastSnapshotOffset } from "./compare-tui.mjs";

const [rawPath, metaPath, colsArg, rowsArg] = process.argv.slice(2);
const cols = Number.parseInt(colsArg ?? "100", 10);
const rows = Number.parseInt(rowsArg ?? "30", 10);
const raw = fs.readFileSync(rawPath);
const offset = lastSnapshotOffset(metaPath);
const SYNC = Buffer.from("\u001b[?2026l");
const ends = [];
let idx = raw.indexOf(SYNC);
while (idx >= 0) { ends.push(idx + SYNC.length); idx = raw.indexOf(SYNC, idx + SYNC.length); }
let prev = 0;
let prevFirst = null;
console.log(`# ${rawPath} 偏移=${offset} 帧数=${ends.length}`);
console.log("帧序号 帧尾偏移 帧字节数 换行\\n 回车换行\\r\\n 光标上移序列 清行数 有效行数 上滚 首行内容");
for (let i = 0; i < ends.length; i += 1) {
  const slice = raw.subarray(prev, ends[i]);
  const text = slice.toString("utf8");
  const newlines = (text.match(/\n/g) ?? []).length;
  const crlf = (text.match(/\r\n/g) ?? []).length;
  const ups = [...text.matchAll(/\u001b\[(\d*)A/g)].map((m) => Number.parseInt(m[1] || "1", 10));
  const clears = (text.match(/\u001b\[2K/g) ?? []).length;
  const lines = normalizeScreen(renderScreen(raw.subarray(0, ends[i]), cols, rows));
  let effective = 0;
  for (let y = 0; y < lines.length; y += 1) if (lines[y].trim() !== "") effective = y + 1;
  // 上滚检测：与上一帧相比"屏幕首行"变了且总内容行数不减 ⇒ 认为发生了上滚（P7-5c/W0）。
  const first = lines.findIndex((l) => l.trim() !== "") >= 0 ? lines.findIndex((l) => l.trim() !== "") : -1;
  const firstText = first >= 0 ? lines[first].slice(0, 24) : "";
  const scrolled = prevFirst !== null && firstText !== prevFirst ? "是" : "";
  prevFirst = firstText;
  console.log(`${String(i + 1).padStart(4)} ${String(ends[i]).padStart(8)} ${String(slice.length).padStart(8)} ${String(newlines).padStart(6)} ${String(crlf).padStart(10)} ${JSON.stringify(ups).padEnd(14)} ${String(clears).padStart(6)} ${String(effective).padStart(6)} ${scrolled.padEnd(4)} ${JSON.stringify(firstText)}`);
  prev = ends[i];
}
