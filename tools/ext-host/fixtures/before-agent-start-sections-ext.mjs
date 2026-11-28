// `before_agent_start` 的 `systemPromptOptions.sections` 夹具。
//
// 契约（ref `types.ts:738-746`、`agent-session.ts:1703-1715`）：
// 事件里必须带 `systemPromptOptions`（其中含 `sections`），处理器的修改
// **回流**到系统提示。真机上 `pi-handoff-gate` 就靠它追加自己的 section
// （`index.ts:557` 的 `event.systemPromptOptions.sections[...] = ...`）——
// 字段缺失时它直接抛 `Cannot read properties of undefined (reading 'sections')`。
export default function beforeAgentStartSections(pi) {
  let seen = "no-event";
  pi.on("before_agent_start", (event) => {
    const options = event && event.systemPromptOptions;
    if (!options || typeof options !== "object") {
      seen = "missing-systemPromptOptions";
      return undefined;
    }
    if (!options.sections || typeof options.sections !== "object") {
      seen = "missing-sections";
      return undefined;
    }
    seen = "ok";
    options.sections["handoff-gate-test"] = "SENTINEL-SECTION-TEXT";
    return { systemPromptOptions: options };
  });
  pi.registerCommand("ba-sections", {
    description: "report what before_agent_start saw",
    handler: () => ({ text: seen }),
  });
}
