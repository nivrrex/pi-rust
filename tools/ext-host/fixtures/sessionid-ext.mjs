// F28/S1 夹具：session_start 里读 `ctx.sessionManager.getSessionId()` 写事件文件。
import fs from "node:fs";

export default function sessionIdExtension(pi) {
  pi.on("session_start", (_event, ctx) => {
    const log = process.env.PI_EVENT_LOG;
    if (!log) return;
    let value;
    try {
      value = ctx?.sessionManager?.getSessionId?.();
      const branch = ctx?.sessionManager?.getBranch?.() ?? [];
      const leaf = ctx?.sessionManager?.getLeafId?.();
      const dir = ctx?.sessionManager?.getSessionDir?.();
      fs.appendFileSync(log, `${JSON.stringify({ branch: branch.length, leaf: leaf ?? null, dir: typeof dir })}\n`);
    } catch (error) {
      value = `error:${String(error?.message ?? error)}`;
    }
    fs.appendFileSync(log, `${JSON.stringify({ id: value ?? null })}\n`);
  });
}
