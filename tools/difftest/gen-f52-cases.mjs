// F52 夹具：合成会话（思考块 + 含换行的超长 bash 命令 + 60 行输出 + edit）。
// 全部合成文本，不含任何真实报告内容。
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";

const chunk = (delta, finish = null, usage) => {
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
const part = (index, id, name, args) =>
  chunk({
    tool_calls: [{ index, id, type: "function", function: { name, arguments: JSON.stringify(args) } }],
  });

// 含换行 + 超长命令行 + `(timeout 600s)` 后缀（ref `renderers/bash.ts:44-47` 的 timeoutSuffix）。
const command = [
  "cat <<'EOF' > /tmp/synth.txt",
  "第一行 CJK 内容用来验证折行",
  "\t缩进行 with tab",
  "EOF",
  "cd /tmp/一个非常长的没有空格的中文目录名字用来测试折行而不是截断/and-a-very-long-ascii-segment-without-any-space-at-all",
  "seq 1 60",
].join("\n");
const lines = [];
lines.push(chunk({ role: "assistant", content: "" }));
lines.push(chunk({ reasoning_content: "先想一下要不要跑合成命令。" }));
lines.push(chunk({ content: "先跑一段合成命令：" }));
lines.push(part(0, "call_bash", "bash", { command, timeout: 600 }));
lines.push(part(1, "call_edit", "edit", { path: "synth.txt", oldText: "旧", newText: "新" }));
lines.push(chunk({}, "tool_calls", { prompt_tokens: 10, completion_tokens: 60, total_tokens: 70 }));

const out = "fixtures/sse/openai/f52-bash-heredoc.sse";
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, lines.join("\n") + "\n");
console.log(out);
