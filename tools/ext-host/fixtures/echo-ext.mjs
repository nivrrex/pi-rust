// 测试扩展：注册 `ext_echo`（回显）、`ext_boom`（抛错）、`ext_sleep`（超时用）。
// 顶部故意 `console.log`：验证"扩展的 stdout 污染"伤不到控制通道（S5 决策的直接证据）。
console.log("[noise] extension stdout is NOT the protocol channel");

export default function echoExtension(pi) {
  pi.on("session_start", () => {});
  pi.registerCommand("greet");
  pi.registerTool({
    name: "ext_echo",
    description: "回显文本（扩展宿主测试用）",
    parameters: {
      type: "object",
      properties: { text: { type: "string" } },
      required: ["text"],
    },
    async execute(_toolCallId, args) {
      return { content: [{ type: "text", text: `echo:${args?.text ?? ""}` }] };
    },
  });
  pi.registerTool({
    name: "ext_sleep",
    description: "睡 N 毫秒（测试超时）",
    parameters: { type: "object", properties: { ms: { type: "number" } } },
    async execute(_toolCallId, args) {
      const ms = Number(args?.ms ?? 200);
      await new Promise((resolve) => setTimeout(resolve, ms));
      return { content: [{ type: "text", text: `slept:${ms}` }] };
    },
  });
  pi.registerTool({
    name: "ext_boom",
    description: "总是抛错（测试错误路径）",
    parameters: { type: "object", properties: {} },
    async execute() {
      throw new Error("boom");
    },
  });
}
