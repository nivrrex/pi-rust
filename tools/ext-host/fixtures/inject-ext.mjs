// `before_agent_start` 返回值夹具（P9-1-E3b / S3）：
// 注入一条自定义消息，并覆盖系统提示。
export default function injectExtension(pi) {
  pi.on("before_agent_start", () => ({
    message: { customType: "note", content: "INJECTED-NOTE" },
    systemPrompt: "CUSTOM SYSTEM PROMPT",
  }));
}
