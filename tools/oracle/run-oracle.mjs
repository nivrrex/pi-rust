#!/usr/bin/env node
// tools/oracle/run-oracle.mjs —— 用 TS 版 pi-ai 生成 SSE 解析的期望结果（oracle）。
//
// 用法（必须包在 unshare 网络命名空间里，见 tools/oracle/run.sh）：
//   PI_AI_PKG=<pi-ai 包目录> node tools/oracle/run-oracle.mjs <manifest.json>
//
// 做法：
//   1. 起一个只监听 127.0.0.1 的 HTTP 服务，把夹具字节按 .chunks.json 分块写出；
//   2. 用 npm 包里的 pi-ai `stream()` 直接打这个服务；
//   3. 收集事件流，归一化后写成 <out>/*.expected.json。
//
// 期望结果不是手写的：完全由 TS 版 pi-ai 产生。

import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const PI_AI_PKG = process.env.PI_AI_PKG;
if (!PI_AI_PKG) {
  console.error("PI_AI_PKG 未设置");
  process.exit(2);
}

const manifestPath = process.argv[2];
if (!manifestPath) {
  console.error("用法: run-oracle.mjs <manifest.json>");
  process.exit(2);
}

const manifest = JSON.parse(await fs.readFile(manifestPath, "utf-8"));
const outDir = path.resolve(manifest.outDir);
await fs.mkdir(outDir, { recursive: true });

const modelFor = (api, baseUrl) => ({
  id: "mock-model",
  name: "Mock Model",
  api,
  provider: "mock",
  baseUrl,
  reasoning: api === "anthropic-messages",
  input: ["text"],
  cost: { input: 1, output: 2, cacheRead: 0, cacheWrite: 0 },
  contextWindow: 200000,
  maxTokens: 4096,
});

const normalizeContent = (content) =>
  content.map((c) => {
    if (c.type === "text") return { type: "text", text: c.text };
    if (c.type === "thinking") return { type: "thinking", thinking: c.thinking };
    if (c.type === "toolCall")
      return { type: "toolCall", id: c.id, name: c.name, arguments: c.arguments };
    return { type: c.type };
  });

const normalizeMessage = (m) => {
  if (!m) return null;
  const out = { role: m.role };
  if (m.api) out.api = m.api;
  if (m.provider) out.provider = m.provider;
  if (m.model) out.model = m.model;
  if (m.responseModel !== undefined) out.responseModel = m.responseModel;
  if (m.stopReason !== undefined) out.stopReason = m.stopReason;
  if (m.rawStopReason !== undefined) out.rawStopReason = m.rawStopReason;
  if (m.errorMessage !== undefined) out.errorMessage = m.errorMessage;
  if (m.endTurn !== undefined) out.endTurn = m.endTurn;
  if (m.toolCallId !== undefined) out.toolCallId = m.toolCallId;
  if (m.toolName !== undefined) out.toolName = m.toolName;
  if (m.isError !== undefined) out.isError = m.isError;
  if (Array.isArray(m.content)) out.content = normalizeContent(m.content);
  if (typeof m.content === "string") out.content = m.content;
  if (m.usage) {
    out.usage = {
      input: m.usage.input,
      output: m.usage.output,
      cacheRead: m.usage.cacheRead,
      cacheWrite: m.usage.cacheWrite,
      totalTokens: m.usage.totalTokens,
      ...(m.usage.reasoning !== undefined ? { reasoning: m.usage.reasoning } : {}),
      // cost 的精确值走文件顶层的 costBits（JSON 小数解析存在 1 ULP 差异，见 notes/questions.md Q1）
      cost: "<COST_BITS>",
    };
  }
  return out;
};

const normalizeEvent = (e) => {
  const out = { type: e.type };
  if (e.contentIndex !== undefined) out.contentIndex = e.contentIndex;
  if (e.delta !== undefined) out.delta = e.delta;
  if (e.content !== undefined) out.content = e.content;
  if (e.reason !== undefined) out.reason = e.reason;
  if (e.toolCall) out.toolCall = normalizeContent([e.toolCall])[0];
  if (e.message) out.message = normalizeMessage(e.message);
  if (e.error) out.error = normalizeMessage(e.error);
  return out;
};

const loadFixture = async (fixturePath) => {
  const bytes = await fs.readFile(fixturePath);
  let chunks = null;
  try {
    const meta = JSON.parse(await fs.readFile(fixturePath + ".chunks.json", "utf-8"));
    chunks = meta.chunks;
  } catch {
    chunks = null;
  }
  if (!chunks) return [bytes];
  const parts = [];
  let offset = 0;
  for (const size of chunks) {
    parts.push(bytes.subarray(offset, offset + size));
    offset += size;
  }
  parts.push(bytes.subarray(offset));
  return parts;
};

const startServer = async (fixturePath) => {
  const chunks = await loadFixture(fixturePath);
  let requests = 0;
  const server = http.createServer(async (req, res) => {
    requests += 1;
    const chunksOut = [];
    req.on("data", (c) => chunksOut.push(c));
    await new Promise((resolve) => req.on("end", resolve));
    res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
    for (const c of chunks) {
      res.write(c);
      await new Promise((r) => setTimeout(r, 1));
    }
    res.end();
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const { port } = server.address();
  return { server, baseUrl: `http://127.0.0.1:${port}`, requestCount: () => requests };
};

const runCase = async (entry) => {
  const fixturePath = path.resolve(entry.fixture);
  const { server, baseUrl, requestCount } = await startServer(fixturePath);
  const apiModule = await import(
    pathToFileURL(path.join(PI_AI_PKG, "dist", "api", `${entry.api}.js`)).href
  );
  const index = await import(pathToFileURL(path.join(PI_AI_PKG, "dist", "index.js")).href);

  const model = modelFor(entry.api, baseUrl);
  const messages = [
    {
      role: "user",
      content: [{ type: "text", text: "hello" }],
      timestamp: 0,
    },
  ];
  const context = index.normalizeContext({ systemPrompt: "You are a mock.", messages });

  const events = [];
  let error = null;
  let rawFinal = null;
  let controller = new AbortController();
  if (entry.abortAfterEvents !== undefined) {
    controller = new AbortController();
  }

  try {
    const stream = apiModule.stream(model, context, {
      apiKey: "mock-key",
      signal: controller.signal,
    });
    let seen = 0;
    for await (const event of stream) {
      events.push(normalizeEvent(event));
      seen += 1;
      if (entry.abortAfterEvents !== undefined && seen >= entry.abortAfterEvents) {
        controller.abort();
      }
    }
    const final = await stream.result();
    rawFinal = final;
    events.push({ type: "__result", message: normalizeMessage(final) });
  } catch (e) {
    error = e instanceof Error ? `${e.name}: ${e.message}` : String(e);
  }

  server.close();
  await new Promise((r) => server.close(r));

  const finalBits = (() => {
    const cost = rawFinal?.usage?.cost;
    if (!cost) return undefined;
    const buf = Buffer.allocUnsafe(8);
    const put = (k) => {
      buf.writeDoubleLE(cost[k]);
      return "0x" + buf.readBigUInt64LE().toString(16).padStart(16, "0");
    };
    return { input: put("input"), output: put("output"), cacheRead: put("cacheRead"), cacheWrite: put("cacheWrite"), total: put("total") };
  })();

  const expected = {
    fixture: entry.fixture,
    ...(finalBits ? { costBits: finalBits } : {}),
    api: entry.api,
    requestCount: requestCount(),
    events,
    ...(error ? { thrown: error } : {}),
  };
  const outFile = path.join(outDir, `${entry.name}.expected.json`);
  await fs.writeFile(outFile, `${JSON.stringify(expected, null, 2)}\n`, "utf-8");
  console.log(`[oracle] ${entry.name}: ${events.length} events -> ${outFile}`);
};

for (const entry of manifest.cases) {
  await runCase(entry);
}