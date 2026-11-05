// F18 夹具：把 session_start / session_shutdown 的 reason 与目标字段追加到 `PI_EVENT_LOG`。
// 只写 {event, reason, hasPrevious, hasTarget}，不含任何本机路径。
import fs from "node:fs";

export default function eventLogExtension(pi) {
  const log = process.env.PI_EVENT_LOG;
  const write = (record) => {
    if (!log) return;
    try {
      fs.appendFileSync(log, `${JSON.stringify(record)}\n`);
    } catch {
      /* 写不进就算了，测试会失败在断言上 */
    }
  };
  pi.on("session_start", (event) => {
    write({
      event: "start",
      reason: event?.reason ?? null,
      hasPrevious: Boolean(event?.previousSessionFile),
      hasTarget: false,
    });
  });
  pi.on("session_shutdown", (event) => {
    write({
      event: "shutdown",
      reason: event?.reason ?? null,
      hasPrevious: false,
      hasTarget: Boolean(event?.targetSessionFile),
    });
  });
}
