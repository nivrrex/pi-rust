// `pi.getSessionName()` 夹具（P9-1-E3b / S4）：未实现会话命名时应为 undefined。
import fs from "node:fs";

export default function sessionNameExtension(pi) {
  pi.on("session_start", () => {
    const log = process.env.PI_EXT_TEST_LOG;
    if (log) fs.appendFileSync(log, `name:${String(pi.getSessionName())}\n`);
  });
}
