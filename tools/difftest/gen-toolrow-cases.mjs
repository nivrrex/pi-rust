// F48/X2：生成工具行折叠的对拍夹具（pending / success / error / 未特判的工具）。
// 全部合成内容；写法对齐既有 `tool-two-calls.sse`（一 chunk 一事件、参数分片）。
import { writeFileSync } from "node:fs";

const head = (delta, finish = null, usage) => {
  const body = {
    id: "chatcmpl-mock",
    object: "chat.completion.chunk",
    created: 1700000000,
    model: "mock-model",
    choices: [{ index: 0, delta, finish_reason: finish }],
  };
  if (usage) body.usage = usage;
  return `data: ${JSON.stringify(body)}\n`;
};

const part = (index, id, name, argsJson) =>
  head({
    tool_calls: [
      { index, id, type: "function", function: { name, arguments: argsJson } },
    ],
  });

const shortOnly = process.argv.includes("--short");
const lines = [
  head({ role: "assistant", content: "" }),
  head({ content: "三个工具行的折叠形态：" }),
  // 1) 成功：read
  part(0, "call_read", "read", '{"path": "alpha.txt"}'),
  // 2) 出错：bash 一条会非零退出但没有输出的命令
  part(1, "call_bash", "bash", '{"command": "false"}'),
  // 3) 未特判的工具名：仍应是单行折叠
  part(2, "call_zeta", "zeta_probe", '{"mode": "x"}'),
  // 4) 超长参数（含 CJK 宽字符）：验证**扩展自己产生的** `…` 位置与 TS 一致。
  //    只在足够宽的终端使用：ref/TS 对扩展行**不做**宽度裁剪，超宽会让 TS
  //    `uncaughtException: Rendered line N exceeds terminal width`（80 列实测崩）。
  ...(shortOnly
    ? []
    : [
        part(
          3,
          "call_long",
          "z",
          JSON.stringify({ note: "很长的中文参数用来验证截断位置".repeat(2) + "x".repeat(60) }),
        ),
      ]),
  head({}, "tool_calls", { prompt_tokens: 10, completion_tokens: 40, total_tokens: 50 }),
];
const out = shortOnly
  ? "fixtures/sse/openai/toolrow-states-short.sse"
  : "fixtures/sse/openai/toolrow-states.sse";
writeFileSync(out, lines.join("\n") + "\n");
console.log(out);
