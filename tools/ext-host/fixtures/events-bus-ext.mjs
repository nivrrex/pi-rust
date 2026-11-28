// F60/20：`pi.events` 共享事件总线探针（ref `event-bus.ts:3-31`）。
//
// 真实使用方（已装扩展）：
// - `pi-handoff-gate`：`pi.events.on(RESILIENCE_PRESENT_CHANNEL, …)`（`index.ts:235-240`）；
// - `pi-agent-resilience`：`pi.events.emit(...)` 广播"我在 / 我要自动恢复"；
// - `pi-interact` ↔ `pi-remote-control`：`pi.remote-control.ui.probe.v1` 同步握手。
// 此前 runner 的 `events` 是 `{publish(){}, on(){return ()=>{}}}` —— 方法名不对且全空，
// 扩展调用 `pi.events.emit` 会直接 TypeError，协作静默失效。
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

export default function eventsBusProbe(pi) {
  pi.on("session_start", (_event) => {
    record(`typeof:emit=${typeof pi.events?.emit}`);
    record(`typeof:on=${typeof pi.events?.on}`);
    // 单向：自己发、自己收（验证 emit/on 真的连通）。
    let seen = 0;
    const unsubscribe = pi.events.on("probe.channel", (data) => {
      seen += 1;
      record(`received:${String(data)}`);
    });
    pi.events.emit("probe.channel", "hello");
    // 退订后不应再收到。
    unsubscribe();
    pi.events.emit("probe.channel", "after-unsubscribe");
    record(`seen=${seen}`);
  });
}
