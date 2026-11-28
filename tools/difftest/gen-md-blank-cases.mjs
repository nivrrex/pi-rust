// 生成 F47/W1 的夹具：块间空行矩阵（源码有/无空行 × 七种相邻块组合）。
// 输出：fixtures/sse/openai/md-blank-{tight,loose}.sse
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";

const PARA = "下面这一段以冒号结尾，用来验证它和后面那块之间是否补空行：";
const PARA2 = "这一小节说明也以冒号结尾，紧接着就是下一块：";

// tight = 相邻两块之间源码**没有**空行；loose = **有**一个空行。
const cases = (blank) => {
  const g = blank ? "\n\n" : "\n";
  return [
    // 段落 → 有序列表
    `${PARA}（段落→有序列表）${g}1. 甲项一\n2. 甲项二`,
    // 段落 → 无序列表
    `${PARA}（段落→无序列表）${g}- 乙项一\n- 乙项二`,
    // 段落 → 代码块
    `${PARA}（段落→代码块）${g}\`\`\`sh\nls -la\n\`\`\``,
    // 代码块 → 列表
    `\`\`\`sh\necho hi\n\`\`\`${g}- 丙项一`,
    // 列表 → 段落
    `- 丁项一${g}${PARA2}（列表→段落）`,
    // 标题 → 列表
    `## 小节标题${g}- 戊项一`,
    // 段落 → 引用
    `${PARA}（段落→引用）${g}> 引用一行`,
  ].join("\n\n");
};

for (const [name, blank] of [
  ["tight", false],
  ["loose", true],
]) {
  const content = cases(blank);
  const chunk = JSON.stringify({
    id: "chatcmpl-mock",
    object: "chat.completion.chunk",
    created: 1700000000,
    model: "mock-model",
    choices: [{ index: 0, delta: { content }, finish_reason: null }],
  });
  const tail = JSON.stringify({
    id: "chatcmpl-mock",
    object: "chat.completion.chunk",
    created: 1700000000,
    model: "mock-model",
    choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
    usage: { prompt_tokens: 10, completion_tokens: 400, total_tokens: 410 },
  });
  const out = `fixtures/sse/openai/md-blank-${name}.sse`;
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, `data: ${chunk}\n\ndata: ${tail}\n\n`);
  console.log(out);
}
