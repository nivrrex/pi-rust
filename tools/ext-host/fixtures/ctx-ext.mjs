// ctx 与 UI 单向帧测试扩展（P9-1-D / S5）：读取只读 ctx，并调用 notify/setStatus。
import fs from "node:fs";

export default function ctxExtension(pi) {
  pi.on("session_start", (event, ctx) => {
    const log = process.env.PI_EXT_TEST_LOG;
    if (log) {
      fs.appendFileSync(
        log,
        `ctx:${ctx?.cwd}:${ctx?.mode}:${ctx?.hasUI}:${ctx?.model ?? "null"}:${ctx?.thinkingLevel ?? "null"}\n`,
      );
    }
    ctx?.ui?.notify?.(`hello from extension (${ctx?.mode})`);
    ctx?.ui?.setStatus?.("demo", "status-from-ext");
  });
}
