// `context_with_system` 夹具：把首条 assistant 文本改写，其它原样。
export default function contextWithSystemProbe(pi) {
  pi.on("context_with_system", (event) => {
    const messages = Array.isArray(event?.messages) ? event.messages : [];
    const seenSystem = messages.some((message) => message?.role === "system");
    return {
      messages: [
        ...messages,
        { role: "user", content: [{ type: "text", text: `SEEN-SYSTEM:${seenSystem}` }] },
      ],
    };
  });
}
