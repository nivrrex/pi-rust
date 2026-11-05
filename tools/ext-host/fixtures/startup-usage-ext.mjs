// F29/S4d 夹具：`session_start` 里读 `ctx.getContextUsage()`，写进 `PI_EVENT_LOG`。
// 存在的意义：真机底栏 `?/0 (auto)` 的根因是"会话开始那一刻扩展读不到 usage"。
import fs from "node:fs";

export default function startupUsageExtension(pi) {
  pi.on("session_start", (_event, ctx) => {
    const log = process.env.PI_EVENT_LOG;
    if (!log) return;
    const usage = ctx?.getContextUsage?.() ?? null;
    fs.appendFileSync(
      log,
      `${JSON.stringify({
        tokens: usage?.tokens ?? null,
        window: usage?.contextWindow ?? null,
        percent: usage?.percent ?? null,
      })}\n`,
    );
  });
}
