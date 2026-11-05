// 无 UI 环境下的 `ctx.ui.select/input/confirm` 语义（ref `runner.ts:320-325` 的 noOpUIContext）。
import fs from "node:fs";

const logPath = process.env.PI_EXT_TEST_LOG;

export default function noUiExtension(pi) {
  pi.on("session_start", async (_event, ctx) => {
    const selected = await ctx.ui.select("Pick one", ["a", "b"]);
    const entered = await ctx.ui.input("Type");
    const confirmed = await ctx.ui.confirm("Sure?");
    if (logPath) {
      fs.appendFileSync(
        logPath,
        `noui:${String(selected)}:${String(entered)}:${String(confirmed)}:hasUI=${String(ctx.hasUI)}\n`,
      );
    }
  });
}
