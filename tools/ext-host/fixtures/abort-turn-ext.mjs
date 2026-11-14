// F60/5b：TUI 里由扩展触发 `ctx.abort()`，验证**回合真的被中止**。
//
// 场景：用户发一条消息（回合开始、屏幕出现 `Working`），扩展在
// `tool_execution_start` 之后调 `ctx.abort()` —— 等价于 `pi-agent-resilience`
// 检测到流卡住时的行为（`index.ts:116/227`）。
//
// 判据（不依赖墙钟）：Work 行最终消失（`agent_settled`）且进程能正常退出。
import fs from "node:fs";

function record(line) {
  const path = process.env.PI_EXT_TEST_LOG;
  if (!path) return;
  try {
    fs.appendFileSync(path, `${line}\n`);
  } catch {
    /* 记录失败不影响扩展 */
  }
}

export default function abortTurnExtension(pi) {
  // 回合一开始就 abort：不依赖任何工具被执行（假 provider 可能不回工具调用）。
  pi.on("agent_start", async (_event, ctx) => {
    record("agent_start");
    try {
      const out = await ctx?.abort?.();
      record(`abort=${JSON.stringify(out ?? null)}`);
    } catch (error) {
      record(`abort_throw=${String(error && error.message ? error.message : error)}`);
    }
  });
  pi.on("agent_settled", () => {
    record("agent_settled");
  });
}
