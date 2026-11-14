// F60/8：`context` 事件探针（请求前的消息变换，ref `runner.ts:1190-1219`）。
//
// 真实使用方是 `context-gateway`（用户已装）：`index.ts:320-` 在 `context` 里做
// **投影层老化**——把重复发送过的工具结果全文换成视图，会话原史不动。
// 此前 pi-rs 完全不派发该事件 ⇒ 老化层从未生效。
//
// 本探针把第一条 user 消息的文本改成 `[aged] <原文>`，并记录被调用次数与消息条数。
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

export default function contextExtension(pi) {
  pi.on("context", (event) => {
    const messages = Array.isArray(event?.messages) ? event.messages : [];
    record(`context:count=${messages.length}`);
    // 不能看到 system 消息（ref 只给非 system）。
    record(`context:has_system=${messages.some((m) => m?.role === "system")}`);
    // 返回**新列表**：把第一条 user 的文本加前缀（验证宿主采用了返回值）。
    const next = messages.map((message, index) => {
      if (index === 0 && message?.role === "user") {
        const content = Array.isArray(message.content) ? message.content : [];
        return {
          ...message,
          content: content.map((part) =>
            part?.type === "text" ? { ...part, text: `[aged] ${part.text}` } : part,
          ),
        };
      }
      return message;
    });
    return { messages: next };
  });
}
