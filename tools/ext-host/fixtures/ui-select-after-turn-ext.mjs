// 一轮结束后的 `ctx.ui.select`（P9 E2 TUI 面板对拍用）。
import fs from "node:fs";

const logPath = process.env.PI_EXT_TEST_LOG;
let asked = false;

export default function selectAfterTurn(pi) {
  pi.on("agent_end", async (_event, ctx) => {
    if (asked) return;
    asked = true;
    const selected = await ctx.ui.select("Pick a color", ["red", "green", "blue"]);
    if (logPath) fs.appendFileSync(logPath, `selected:${String(selected)}\n`);
  });
}
