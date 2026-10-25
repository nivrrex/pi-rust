#!/usr/bin/env node
// tools/oracle/truncate-oracle.mjs —— 直接调用 npm 包里的 truncateHead/truncateTail/formatSize，
// 生成 Rust 侧纯函数必须逐项一致的期望（P3.2）。
//
// 必须在网络隔离内运行（不联网，但保持一致的做法）：tools/oracle/run-truncate.sh

import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const PKG = process.env.PI_CODING_AGENT_INDEX;
const index = await import(pathToFileURL(PKG).href);
const { truncateHead, truncateTail, formatSize } = index;
for (const [name, fn] of Object.entries({ truncateHead, truncateTail, formatSize })) {
  if (typeof fn !== "function") throw new Error(`包根没有导出 ${name}`);
}

const lines = (n, prefix = "line") => Array.from({ length: n }, (_, i) => `${prefix}${i + 1}`).join("\n");

const CASES = {
  "empty": { content: "", maxLines: 10, maxBytes: 100 },
  "one-line": { content: "a", maxLines: 10, maxBytes: 100 },
  "trailing-newline": { content: "a\nb\n", maxLines: 10, maxBytes: 100 },
  "exact-lines": { content: lines(3), maxLines: 3, maxBytes: 1000 },
  "over-lines-by-one": { content: lines(4), maxLines: 3, maxBytes: 1000 },
  "far-over-lines": { content: lines(50), maxLines: 5, maxBytes: 100000 },
  "exact-bytes": { content: "aaaa\nbbbb", maxLines: 10, maxBytes: 9 },
  "over-bytes": { content: "aaaa\nbbbb", maxLines: 10, maxBytes: 8 },
  "first-line-exceeds": { content: "x".repeat(40), maxLines: 10, maxBytes: 10 },
  "first-line-exact": { content: "x".repeat(10), maxLines: 10, maxBytes: 10 },
  "multibyte-mix": { content: "中文中文\nabcd\n中文", maxLines: 10, maxBytes: 14 },
  "multibyte-first-line-exceeds": { content: "中".repeat(20), maxLines: 5, maxBytes: 20 },
  "blank-lines": { content: "a\n\n\nb", maxLines: 10, maxBytes: 100 },
  "crlf": { content: "a\r\nb\r\nc", maxLines: 2, maxBytes: 100 },
  "no-trailing-newline-many": { content: lines(20), maxLines: 7, maxBytes: 100000 },
  "bytes-limit-hits-mid-list": { content: lines(20, "abcdefghij"), maxLines: 100, maxBytes: 45 },
  "single-huge-line-tail": { content: "z".repeat(100), maxLines: 10, maxBytes: 20 },
  "tail-keeps-end": { content: lines(10), maxLines: 3, maxBytes: 100000 },
  "tail-bytes": { content: lines(10, "0123456789"), maxLines: 100, maxBytes: 35 },
  "tail-empty": { content: "", maxLines: 10, maxBytes: 100 },
  "tail-trailing-newline": { content: "a\nb\n", maxLines: 10, maxBytes: 100 },
  "tail-multibyte": { content: "中文\nabc\n中文", maxLines: 10, maxBytes: 13 },
  "tail-exact-lines": { content: lines(3), maxLines: 3, maxBytes: 1000 },
  "tail-over-lines": { content: lines(4), maxLines: 3, maxBytes: 1000 },
  "tail-blank-lines": { content: "a\n\n\nb", maxLines: 10, maxBytes: 100 },
  "spaces": { content: "   \n\t\nx", maxLines: 10, maxBytes: 100 },
  "one-char-per-line": { content: "a\nb\nc\nd\ne", maxLines: 3, maxBytes: 100 },
  "maxlines-zero": { content: lines(3), maxLines: 0, maxBytes: 100 },
  "maxbytes-zero": { content: "abc", maxLines: 10, maxBytes: 0 },
  "large-file": { content: lines(5000), maxLines: 2000, maxBytes: 50 * 1024 },
  "large-file-tail": { content: lines(5000), maxLines: 2000, maxBytes: 50 * 1024 },
  "long-lines": { content: Array.from({ length: 5 }, () => "y".repeat(1000)).join("\n"), maxLines: 3, maxBytes: 4096 },
  "mixed-widths": { content: "短\nvery long line here\n中\nz", maxLines: 2, maxBytes: 100 },
  "just-newlines": { content: "\n\n\n", maxLines: 10, maxBytes: 100 },
  "windows-crlf-bytes": { content: "aaaa\r\nbbbb\r\ncccc", maxLines: 10, maxBytes: 14 },
};

const out = {};
for (const [name, spec] of Object.entries(CASES)) {
  const { content, maxLines, maxBytes } = spec;
  const head = truncateHead(content, { maxLines, maxBytes });
  const tail = truncateTail(content, { maxLines, maxBytes });
  const pick = (r) => ({
    content: r.content,
    truncated: r.truncated,
    truncatedBy: r.truncatedBy ?? null,
    totalLines: r.totalLines,
    totalBytes: r.totalBytes,
    outputLines: r.outputLines,
    outputBytes: r.outputBytes,
    firstLineExceedsLimit: r.firstLineExceedsLimit ?? null,
    lastLinePartial: r.lastLinePartial ?? null,
  });
  out[name] = { content, maxLines, maxBytes, head: pick(head), tail: pick(tail) };
}

await fs.mkdir(path.resolve("fixtures/truncate"), { recursive: true });
await fs.writeFile(path.resolve("fixtures/truncate/cases.json"), `${JSON.stringify(out, null, 2)}\n`);
console.log(`[truncate-oracle] 生成 ${Object.keys(out).length} 组用例`);
console.log(`[truncate-oracle] formatSize 样例: 999=>${formatSize(999)}, 1024=>${formatSize(1024)}, 1536=>${formatSize(1536)}, 51200=>${formatSize(51200)}`);
