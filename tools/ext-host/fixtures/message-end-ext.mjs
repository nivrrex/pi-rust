// F60/18：`message_end` 事件探针（ref `agent-session.ts:1093-1099`）。
//
// 已装扩展有 4 处订阅它：
// - `pi-agent-resilience`（`index.ts:234`）：响应结束复位看门狗/预算；
// - `pi-ai-summary`（`src/events.ts:186`）：累计 token 与费用；
// - `context-gateway`（`index.ts:97`）：`telemetry.response(event.message)`。
// 此前 pi-rs 完全不派发 ⇒ 上述逻辑从未执行。
import fs from "node:fs";

export default function messageEndProbe(pi) {
  pi.on("message_end", (event) => {
    const path = process.env.PI_EXT_TEST_LOG;
    if (!path) return;
    const message = event?.message ?? {};
    const role = typeof message.role === "string" ? message.role : "<none>";
    try {
      fs.appendFileSync(path, `message_end:${role}\n`);
    } catch {
      /* 记录失败不影响扩展 */
    }
  });
}
