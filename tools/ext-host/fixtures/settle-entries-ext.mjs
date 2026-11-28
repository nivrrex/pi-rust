// `agent_before_settle` 的 `entries` 夹具（P9-1-E3b / S3）：返回一条 custom 条目并续跑一轮。
let rounds = 0;

export default function settleEntriesExtension(pi) {
  pi.on("agent_before_settle", () => {
    rounds += 1;
    if (rounds > 1) return { continue: false };
    return {
      continue: true,
      entries: [{ customType: "settle-note", data: "ENTRY-DATA" }],
    };
  });
}
