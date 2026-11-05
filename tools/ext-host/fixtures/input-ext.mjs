// `input` / `before_agent_start` / `agent_before_settle` 夹具（P9 E2）。
// - `input`：`!drop` 开头 ⇒ handled（不跑 agent）；`!upper ` 开头 ⇒ transform 成大写；
// - `before_agent_start`：记录一次，并返回一条 message；
// - `agent_before_settle`：记录一次，返回 `continue:false`。
import fs from "node:fs";

const logPath = process.env.PI_EXT_TEST_LOG;

function record(line) {
  if (!logPath) return;
  fs.appendFileSync(logPath, `${line}\n`);
}

export default function inputExtension(pi) {
  pi.on("input", (event) => {
    record(`input:${event?.source ?? "?"}:${String(event?.text ?? "")}`);
    const text = String(event?.text ?? "");
    if (text.startsWith("!drop")) return { action: "handled" };
    if (text.startsWith("!upper ")) {
      return { action: "transform", text: text.slice("!upper ".length).toUpperCase() };
    }
    return { action: "continue" };
  });
  pi.on("before_agent_start", (event) => {
    record(`before_agent_start:${String(event?.prompt ?? "")}`);
    return { message: { customType: "e2", content: "from-before-agent-start" } };
  });
  pi.on("agent_before_settle", (event) => {
    record(`agent_before_settle:${String(event?.outcome ?? "?")}`);
    return { continue: false };
  });
}
