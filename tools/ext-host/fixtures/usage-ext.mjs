// `ctx.getContextUsage()` 夹具（P9-1-E3b / S4）：把宿主下发的用量写进日志。
import fs from "node:fs";

const logPath = process.env.PI_EXT_TEST_LOG;

export default function usageExtension(pi) {
  pi.on("agent_end", (_event, ctx) => {
    if (!logPath) return;
    const usage = ctx.getContextUsage();
    fs.appendFileSync(
      logPath,
      `usage:${String(usage?.tokens)}:${String(usage?.contextWindow)}:${String(usage?.percent)}\n`,
    );
  });
}
