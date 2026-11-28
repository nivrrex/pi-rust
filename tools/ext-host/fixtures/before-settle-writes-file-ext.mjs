// `agent_before_settle` 端到端探针：每次被调用就往文件追加一行。
// 用文件而不是返回值：TUI 路径的返回值目前只落盘 entries（不续跑）。
import fs from "node:fs";
import path from "node:path";

export default function beforeSettleWritesFile(pi) {
  pi.on("agent_before_settle", (event) => {
    try {
      const dir = process.env.PI_CODING_AGENT_DIR ?? process.cwd();
      fs.appendFileSync(path.join(dir, "settle-calls.log"), `call:${event?.outcome ?? "?"}\n`);
    } catch {
      /* 写不进不影响判定 */
    }
    return undefined;
  });
}
