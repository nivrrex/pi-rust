// `ctx.sessionManager` 只读子集夹具（P9-1-E3b / S4）。
import fs from "node:fs";

export default function sessionManagerExtension(pi) {
  pi.on("agent_end", (_event, ctx) => {
    const log = process.env.PI_EXT_TEST_LOG;
    if (!log) return;
    const entries = ctx.sessionManager.getEntries();
    const file = ctx.sessionManager.getSessionFile();
    fs.appendFileSync(log, `sm:${entries.length}:${file ? "file" : "nofile"}\n`);
  });
}
