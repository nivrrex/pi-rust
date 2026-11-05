// `session_start` 的 reason 记录器（P9-1-E1c / S2b、S3）。
// 与 `event-ext.mjs` 的区别：**不退订**，因此同一进程里的多次 `session_start`
// （startup → resume 等）都会被记录，顺序即派发顺序。
import fs from "node:fs";

const logPath = process.env.PI_EXT_TEST_LOG;

function record(line) {
  if (!logPath) return;
  fs.appendFileSync(logPath, `${line}\n`);
}

export default function reasonExtension(pi) {
  pi.on("session_start", (event) => {
    record(`session_start:${event?.reason ?? "?"}`);
  });
  pi.on("agent_end", (event) => {
    const count = Array.isArray(event?.messages) ? event.messages.length : -1;
    record(`agent_end:${count}`);
  });
}
