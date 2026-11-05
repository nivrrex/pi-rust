// 事件分发测试扩展（P9-1-C / S5）：
// - `session_start`：记录一次，然后**自己退订**（验证 unsubscribe 生效）；
// - `agent_end`：记录后抛错（验证"处理器抛错不中断主流程"）。
import fs from "node:fs";

const logPath = process.env.PI_EXT_TEST_LOG;

function record(line) {
  if (!logPath) return;
  fs.appendFileSync(logPath, `${line}\n`);
}

export default function eventExtension(pi) {
  let unsubscribe = () => {};
  unsubscribe = pi.on("session_start", (event) => {
    record(`session_start:${event?.reason ?? "?"}`);
    unsubscribe();
  });
  pi.on("agent_end", (event) => {
    const count = Array.isArray(event?.messages) ? event.messages.length : -1;
    record(`agent_end:${count}`);
    throw new Error("boom-in-agent_end");
  });
  // 超时测试用：处理器故意慢（500 ms），宿主用更短的超时分发。
  pi.on("slow_event", async () => {
    await new Promise((resolve) => setTimeout(resolve, 500));
    record("slow_event:done");
  });
  pi.registerTool({
    name: "ext_ping",
    description: "存活探测（事件测试用）",
    parameters: { type: "object", properties: {} },
    async execute() {
      record("ping");
      return { content: [{ type: "text", text: "pong" }] };
    },
  });
}
