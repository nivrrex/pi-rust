#!/usr/bin/env node
// tools/oracle/anthropic-request-oracle.mjs —— 让 TS 的 pi-ai 对本地 mock 发出多种上下文，
// 把 mock 捕获到的请求体存为夹具，供 Rust 侧逐形态比对。
//
// 必须在网络隔离内运行（见 tools/oracle/run-anthropic-request.sh）。
// 期望值完全由 TS 产生，不手写。

import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const PI_AI_PKG = process.env.PI_AI_PKG;
if (!PI_AI_PKG) {
  console.error("PI_AI_PKG 未设置");
  process.exit(2);
}

const OUT = path.resolve("fixtures/anthropic-requests");

const TEXT = (text) => ({ type: "text", text });
const USER = (content, timestamp = 0) => ({ role: "user", content, timestamp });
// assistant 消息要带 model：transform-messages.ts:109 只有 isSameModel 时才原样保留 thinking。
const ASSISTANT = (content, timestamp = 0) => ({
  role: "assistant",
  content,
  timestamp,
  model: "mock-model",
  api: "anthropic-messages",
  provider: "mock",
});
// TS 的输入模型里 toolResult 是**独立的一条消息**（types.ts:538-548），
// 请求体里的 user+tool_result 块是由 transform-messages.ts 转换出来的。
const TOOL_RESULT = (toolCallId, toolName, content, isError = false, timestamp = 0) => ({
  role: "toolResult",
  toolCallId,
  toolName,
  content,
  isError,
  timestamp,
});

/** 每种上下文的构造方式；名字与 Rust 侧测试一一对应。 */
const CASES = {
  "system-only": () => ({
    systemPrompt: "You are a mock.",
    messages: [USER([TEXT("hello")])],
  }),
  "no-system": () => ({
    systemPrompt: undefined,
    messages: [USER([TEXT("hello")])],
  }),
  "multi-turn": () => ({
    systemPrompt: "sys",
    messages: [
      USER([TEXT("first")]),
      ASSISTANT([TEXT("reply")]),
      USER([TEXT("second")]),
    ],
  }),
  "with-tools": () => ({
    systemPrompt: "sys",
    messages: [USER([TEXT("use a tool")])],
    tools: [
      {
        name: "read",
        description: "Read a file.",
        parameters: { type: "object", properties: { path: { type: "string" } }, required: ["path"] },
      },
    ],
  }),
  "tool-use-and-result": () => ({
    systemPrompt: "sys",
    messages: [
      USER([TEXT("read it")]),
      ASSISTANT([{ type: "toolCall", id: "call_1", name: "read", arguments: { path: "a.txt" } }]),
      TOOL_RESULT("call_1", "read", [TEXT("file body")]),
    ],
  }),
  "with-image": () => ({
    systemPrompt: "sys",
    messages: [
      USER([TEXT("look"), { type: "image", mediaType: "image/png", data: "iVBORw0KGgo=" }]),
    ],
  }),
  "image-in-tool-result": () => ({
    systemPrompt: "sys",
    messages: [
      USER([TEXT("look")]),
      ASSISTANT([{ type: "toolCall", id: "call_1", name: "read", arguments: { path: "p.png" } }]),
      TOOL_RESULT("call_1", "read", [
        TEXT("[image/png]"),
        { type: "image", mediaType: "image/png", data: "iVBORw0KGgo=" },
      ]),
    ],
  }),
  "with-thinking": () => ({
    systemPrompt: "sys",
    messages: [
      USER([TEXT("think")]),
      ASSISTANT([{ type: "thinking", thinking: "hmm", thinkingSignature: "sig" }]),
    ],
    reasoning: "medium",
  }),
  "thinking-only-request": () => ({
    systemPrompt: "sys",
    messages: [USER([TEXT("think")])],
    reasoning: "high",
  }),
  "cache-retention": () => ({
    systemPrompt: "sys",
    messages: [
      USER([TEXT("one")]),
      ASSISTANT([TEXT("two")]),
      USER([TEXT("three")]),
    ],
    cacheRetention: "24h",
  }),
};

const model = (baseUrl, reasoning) => ({
  id: "mock-model",
  name: "Mock Model",
  api: "anthropic-messages",
  provider: "mock",
  baseUrl,
  reasoning,
  input: ["text", "image"],
  cost: { input: 1, output: 2, cacheRead: 0, cacheWrite: 0 },
  contextWindow: 200000,
  maxTokens: 4096,
});

/** 最小可用的 anthropic SSE 响应，让 stream() 能正常结束。 */
const MINIMAL_SSE = [
  'event: message_start',
  'data: {"type":"message_start","message":{"id":"m","type":"message","role":"assistant","model":"mock-model","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":0}}}',
  "",
  'event: content_block_start',
  'data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
  "",
  'event: content_block_delta',
  'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}',
  "",
  'event: content_block_stop',
  'data: {"type":"content_block_stop","index":0}',
  "",
  'event: message_delta',
  'data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}',
  "",
  'event: message_stop',
  'data: {"type":"message_stop"}',
  "",
  "",
].join("\n");

const captured = [];
const server = http.createServer((req, res) => {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => {
    const body = Buffer.concat(chunks).toString("utf-8");
    captured.push({ path: req.url, headers: req.headers, body });
    res.writeHead(200, { "content-type": "text/event-stream" });
    res.end(MINIMAL_SSE);
  });
});

await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const baseUrl = `http://127.0.0.1:${server.address().port}`;

// stream() 在按 api 拆分的模块里；normalizeContext() 在包的主入口。
const apiModule = await import(pathToFileURL(path.join(PI_AI_PKG, "dist", "api", "anthropic-messages.js")).href);
const index = await import(pathToFileURL(path.join(PI_AI_PKG, "dist", "index.js")).href);
await fs.mkdir(OUT, { recursive: true });

for (const [name, build] of Object.entries(CASES)) {
  const spec = build();
  const before = captured.length;
  const context = index.normalizeContext({
    ...(spec.systemPrompt === undefined ? {} : { systemPrompt: spec.systemPrompt }),
    messages: spec.messages,
    ...(spec.tools ? { tools: spec.tools } : {}),
  });


  let error = null;
  try {
    const stream = apiModule.stream(model(baseUrl, spec.reasoning !== undefined), context, {
      apiKey: "mock-key",
      ...(spec.reasoning !== undefined ? { reasoning: spec.reasoning } : {}),
      ...(spec.cacheRetention !== undefined ? { cacheRetention: spec.cacheRetention } : {}),
    });
    for await (const _event of stream) {
      void _event;
    }
    await stream.result();
  } catch (e) {
    error = e instanceof Error ? `${e.name}: ${e.message}` : String(e);
  }

  const hit = captured.slice(before)[0];
  const out = {
    name,
    input: { systemPrompt: spec.systemPrompt ?? null, hasTools: Boolean(spec.tools), reasoning: spec.reasoning ?? null, cacheRetention: spec.cacheRetention ?? null },
    error,
    path: hit?.path ?? null,
    // 只留判据相关的头，避免把随机 id / date 之类写进夹具。
    headers: hit
      ? Object.fromEntries(
          Object.entries(hit.headers).filter(([key]) =>
            ["anthropic-beta", "anthropic-version", "x-api-key", "authorization", "content-type"].includes(
              key.toLowerCase(),
            ),
          ),
        )
      : null,
    body: hit ? JSON.parse(hit.body) : null,
  };
  await fs.writeFile(path.join(OUT, `${name}.json`), `${JSON.stringify(out, null, 2)}\n`);
  console.log(`[anthropic-request] ${name}: ${error ? `错误 ${error}` : `path=${out.path} 顶层键=${Object.keys(out.body ?? {}).join(",")}`}`);
}
await fs.writeFile(path.join(OUT, "index.json"), `${JSON.stringify(Object.keys(CASES), null, 2)}\n`);
server.close();
