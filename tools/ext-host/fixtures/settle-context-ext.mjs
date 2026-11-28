// `agent_before_settle` 的 `context` 载荷夹具。
//
// 契约（ref `types.ts:797-819` 的 `BoundaryContextPreview` + `BoundaryState`）：
// 事件里必须带 `context.llmMessages`，`pi-handoff-gate`/`pi-agent-resilience`
// 都靠它判断"最后一条是不是空 assistant"，缺失时直接抛
// `Cannot read properties of undefined (reading 'llmMessages')`。
export default function settleContextProbe(pi) {
  let seen = "no-event";
  pi.on("agent_before_settle", (event) => {
    const context = event && event.context;
    if (!context || typeof context !== "object") {
      seen = "missing-context";
      return undefined;
    }
    if (!Array.isArray(context.llmMessages)) {
      seen = "missing-llmMessages";
      return undefined;
    }
    seen = `ok:${context.llmMessages.length}`;
    return undefined;
  });
  // 宿主侧的 `ext.command.invoke` 只判成败、不取返回值（与既有约定一致），
  // 所以把观察结果同时写文件，供测试读。
  pi.registerCommand("settle-context", {
    description: "report what agent_before_settle saw",
    handler: () => {
      try {
        const fs = process.getBuiltinModule
          ? process.getBuiltinModule("node:fs")
          : null;
        if (fs && process.env.PI_SETTLE_PROBE_LOG) {
          fs.appendFileSync(process.env.PI_SETTLE_PROBE_LOG, `${seen}\n`);
        }
      } catch {}
      return { text: seen };
    },
  });
}
