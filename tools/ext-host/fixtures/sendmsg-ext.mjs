// `pi.sendUserMessage` 夹具（P9-1-E3b / S4）：在 `agent_end` 里排入一条用户消息，
// 验证 print 路径的 `agent_end` 派发早于 settle（否则排的消息赶不上本轮）。
let sent = false;

export default function sendMessageExtension(pi) {
  pi.on("agent_end", () => {
    if (sent) return;
    sent = true;
    pi.sendUserMessage("SECOND-TURN");
  });
}
