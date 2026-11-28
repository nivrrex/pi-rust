// 阻塞式 UI 询问夹具：`session_start` 里调 `ctx.ui.select`，把结果写日志。
import fs from "node:fs";

const logPath = process.env.PI_EXT_TEST_LOG;

export default function selectExtension(pi) {
  pi.on("session_start", async (_event, ctx) => {
    const selected = await ctx.ui.select("Pick a color", ["red", "green", "blue"]);
    const text = await ctx.ui.input("Your name");
    const ok = await ctx.ui.confirm("Continue?");
    if (logPath) {
      fs.appendFileSync(logPath, `selected:${String(selected)}:${String(text)}:${String(ok)}\n`);
    }
  });
}
