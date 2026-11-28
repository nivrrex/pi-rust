#!/usr/bin/env node
// 直接调用官方 OutputAccumulator 生成 Rust 侧的快照期望。

import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const entry = process.env.TS_PI_ENTRY;
if (!entry) throw new Error("TS_PI_ENTRY is required");
const modulePath = path.resolve(path.dirname(path.dirname(entry)), "core/tools/output-accumulator.js");
const { OutputAccumulator } = await import(pathToFileURL(modulePath).href);
if (typeof OutputAccumulator !== "function") throw new Error("official OutputAccumulator not found");

const bytes = (text) => Array.from(Buffer.from(text, "utf8"));
const cases = {
  empty: { chunks: [], maxLines: 10, maxBytes: 100 },
  small: { chunks: [bytes("hello\n")], maxLines: 10, maxBytes: 100 },
  "trailing-newline": { chunks: [bytes("a\nb\n")], maxLines: 10, maxBytes: 100 },
  "over-lines": { chunks: [bytes("one\ntwo\nthree\n")], maxLines: 2, maxBytes: 100 },
  "single-long-line": { chunks: [bytes("x".repeat(100))], maxLines: 10, maxBytes: 20 },
  "split-multibyte": {
    chunks: [[0xe4], [0xb8, 0xad], [0x0a]],
    maxLines: 10,
    maxBytes: 100,
  },
  "invalid-utf8": { chunks: [bytes("a"), [0xff], bytes("b")], maxLines: 10, maxBytes: 100 },
  "mixed-chunks": {
    chunks: [bytes("one\n"), bytes("two\n"), bytes("three\n")],
    maxLines: 2,
    maxBytes: 100,
  },
  "exact-limits": { chunks: [bytes("aa\nbb")], maxLines: 2, maxBytes: 5 },
};

const out = {};
for (const [name, spec] of Object.entries(cases)) {
  const output = new OutputAccumulator({
    maxLines: spec.maxLines,
    maxBytes: spec.maxBytes,
    tempFilePrefix: "pi-output-oracle",
  });
  for (const chunk of spec.chunks) output.append(Buffer.from(chunk));
  output.finish();
  const snapshot = output.snapshot({ persistIfTruncated: true });
  await output.closeTempFile();
  let fullOutput;
  if (snapshot.fullOutputPath) {
    fullOutput = Array.from(await fs.readFile(snapshot.fullOutputPath));
    await fs.rm(snapshot.fullOutputPath, { force: true });
  }
  out[name] = {
    chunks: spec.chunks,
    maxLines: spec.maxLines,
    maxBytes: spec.maxBytes,
    content: snapshot.content,
    truncation: snapshot.truncation,
    lastLineBytes: output.getLastLineBytes(),
    ...(fullOutput === undefined ? {} : { fullOutput }),
  };
}

await fs.mkdir(path.resolve("fixtures/output-accumulator"), { recursive: true });
await fs.writeFile(
  path.resolve("fixtures/output-accumulator/cases.json"),
  `${JSON.stringify(out, null, 2)}\n`,
);
console.log(`[output-accumulator-oracle] generated ${Object.keys(out).length} cases`);
