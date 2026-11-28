// `agent_before_settle` 续跑探针：前 2 次返回 continue 并附一条催促条目。
import fs from "node:fs";
import path from "node:path";

const MAX_CONTINUE = 2;

export default function settleContinue(pi) {
  let calls = 0;
  pi.on("agent_before_settle", () => {
    calls += 1;
    try {
      const dir = process.env.PI_CODING_AGENT_DIR ?? process.cwd();
      fs.appendFileSync(path.join(dir, "settle-calls.log"), `call:${calls}\n`);
    } catch {
      /* 写不进不影响判定 */
    }
    if (calls <= MAX_CONTINUE) {
      return { continue: true, entries: [{ customType: "handoff-gate", data: `CONTINUE-NUDGE-${calls}` }] };
    }
    return undefined;
  });
}
