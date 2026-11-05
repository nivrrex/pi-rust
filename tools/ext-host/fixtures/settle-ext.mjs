// `agent_before_settle` 的"再跑一轮"夹具（P9-1-E3b / S3）：
// 第一次 settle 时返回 `continue: true`，第二次返回 false ⇒ 恰好两轮。
let rounds = 0;

export default function settleExtension(pi) {
  pi.on("agent_before_settle", () => {
    rounds += 1;
    return { continue: rounds === 1 };
  });
}
