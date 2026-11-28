// 钩子测试扩展（P9-1-D / S4）：
// - `tool_call`：`input.blockViaHook === true` 时 block（理由固定）；
// - `tool_result`：两个处理器链式改写（验证顺序：先 -1 再 -2），
//   并把包含 `SECRET` 的文本替换成 `[redacted]`（模拟 pi-secrets 的脱敏语义）。
export default function hookExtension(pi) {
  pi.on("tool_call", (event) => {
    if (event?.input?.blockViaHook === true) {
      return { block: true, reason: "blocked-by-ext" };
    }
    return undefined;
  });
  pi.on("tool_result", (event) => {
    const text = Array.isArray(event?.content)
      ? event.content.filter((b) => b?.type === "text").map((b) => b.text).join("")
      : "";
    if (!text) return undefined;
    const redacted = text.replaceAll("SECRET", "[redacted]");
    return { content: [{ type: "text", text: redacted }] };
  });
  pi.on("tool_result", (event) => {
    // 第二个处理器看到的是上一步改写后的内容 ⇒ 追加 `-2`（顺序证据）。
    const text = Array.isArray(event?.content)
      ? event.content.filter((b) => b?.type === "text").map((b) => b.text).join("")
      : "";
    return { content: [{ type: "text", text: `${text}-2` }] };
  });
}
