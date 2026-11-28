# fixtures/sse — SSE 夹具说明

供 G0（B 段）与 P1 的 mock-provider 共用。每组夹具是**要原样发出的字节**，不是"逻辑事件列表"，
这样 CRLF、跨块 UTF-8、多行 data 这类边界才能被真实复现。

## 文件

- `*.sse`：完整响应体字节，`Content-Type: text/event-stream`，**包含结尾的空行**（即以一个空行结束）。
- `*.sse.chunks.json`（可选）：`{"chunks": [字节数, ...]}`，服务端按这些字节长度分块写出。
  只有一个夹具用到（`openai/multibyte-split.sse`），目的是让一个 3 字节 UTF-8 字符被切在两个网络块之间。

## 已覆盖的边界

**OpenAI Chat Completions（`openai/`）**

| 夹具 | 覆盖点 |
|---|---|
| `text-basic.sse` | 纯文本回复、`finish_reason: "stop"`、`usage`、`data: [DONE]` |
| `reasoning.sse` | `delta.reasoning_content` 推理字段、`completion_tokens_details.reasoning_tokens` |
| `parallel-toolcalls.sse` | 两个并行 tool call，`index` 交错出现（先 1 后 0），`arguments` 分片交错 |
| `crlf.sse` | 全部行以 `\r\n` 结尾 |
| `multiline-data.sse` | 一个事件用两条 `data:` 行承载，断在逗号边界，拼接后仍是合法 JSON |
| `multibyte-split.sse` | 多字节字符被切在两个网络块之间（配合 `.chunks.json`） |
| `error-chunk.sse` | 流中只发一个 `error` 载荷，无 `[DONE]` |

**Anthropic Messages（`anthropic/`）**

| 夹具 | 覆盖点 |
|---|---|
| `basic.sse` | `message_start`、`content_block_start/delta/stop`、`ping`、`message_delta`、`message_stop` |
| `tool-use-delta.sse` | text 块 + `tool_use` 块，`input_json_delta` 把 `input` 分两片下发 |
| `ping-only.sse` | 最小流：`message_start` 后紧跟 `ping`，再走完整文本块 |

## 约定

- 所有夹具里的 `id`/`created`/`model` 都是固定假值（`chatcmpl-mock` / `msg_mock` / `mock-model`），
  不含时间戳或随机量，便于期望结果逐字节比对。
- `usage` 里的 token 数是手写的固定值，**不是**真实计费结果；期望 JSON 里的 `cost` 由 pi-ai 的
  `calculateCost` 依据模型费率算出，因此模型对象的费率字段也必须固定。
- 新增夹具时同步在 `tools/oracle/manifest.json` 登记，否则 oracle 不会生成期望结果。