// P7-6g/W1：只读分析——按 `?2026l` 帧统计 raw 里的转义序列构成（用于 #13 机制诊断）。
//
// 用法：node tools/difftest/analyze-raw.mjs <raw.bin> [<raw.bin> ...]
// 输出每个文件的逐帧表：帧号/字节数、?2026h/l、2J/3J、\x1b[nA（上移）、2K（清行）、\x1b[J、DECSTBM、\r\n 数、备用屏幕开关。
import fs from "fs";

const SYNC_END = Buffer.from("\u001b[?2026l");

function count(buffer, pattern) {
  return (buffer.toString("latin1").match(pattern) ?? []).length;
}

function analyze(file) {
  const raw = fs.readFileSync(file);
  const ends = [];
  let idx = raw.indexOf(SYNC_END);
  while (idx >= 0) { ends.push(idx + SYNC_END.length); idx = raw.indexOf(SYNC_END, idx + SYNC_END.length); }
  const frames = [];
  let prev = 0;
  for (const end of ends) {
    const chunk = raw.subarray(prev, end);
    frames.push({
      bytes: chunk.length,
      sync: count(chunk, /\u001b\[\?2026[hl]/g),
      clearScreen: count(chunk, /\u001b\[[23]J/g),
      cursorUp: (chunk.toString("latin1").match(/\u001b\[(\d*)A/g) ?? []).map((m) => Number.parseInt(m.slice(2, -1) || "1", 10)),
      clearLine: count(chunk, /\u001b\[2K/g),
      eraseBelow: count(chunk, /\u001b\[0?J/g),
      decstbm: count(chunk, /\u001b\[\d*;\d*r/g),
      crlf: count(chunk, /\r\n/g),
      altScreen: count(chunk, /\u001b\[\?1049[hl]/g),
    });
    prev = end;
  }
  return { file, totalBytes: raw.length, frames };
}

const files = process.argv.slice(2);
if (files.length === 0) { console.error("用法: node tools/difftest/analyze-raw.mjs <raw.bin>..."); process.exit(2); }
const results = files.map(analyze);
// 断言：每个输入文件都被处理（不得空过）。
if (results.length !== files.length) throw new Error("有输入未被处理");
for (const result of results) {
  if (result.frames.length === 0) throw new Error(`${result.file} 没有 ?2026l 帧`);
  console.log(`# ${result.file} 总字节=${result.totalBytes} 帧数=${result.frames.length}`);
  console.log("帧 字节 ?2026 2J/3J 上移 2K [J DECSTBM \\r\\n altScreen");
  result.frames.forEach((f, i) => {
    console.log(`${i + 1} ${f.bytes} ${f.sync} ${f.clearScreen} ${JSON.stringify(f.cursorUp)} ${f.clearLine} ${f.eraseBelow} ${f.decstbm} ${f.crlf} ${f.altScreen}`);
  });
}
