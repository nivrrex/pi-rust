// `registerFlag` / `getFlag` 夹具（P9-1-E3b）：把注册默认值与实际值写进日志。
import fs from "node:fs";

const logPath = process.env.PI_EXT_TEST_LOG;

export default function flagExtension(pi) {
  pi.registerFlag("demo-flag", { description: "示例（字符串）", type: "string", default: "fallback" });
  pi.registerFlag("plain-switch", { description: "布尔开关", type: "boolean", default: false });
  pi.on("session_start", () => {
    if (!logPath) return;
    fs.appendFileSync(
      logPath,
      `flags:${String(pi.getFlag("demo-flag"))}:${String(pi.getFlag("plain-switch"))}\n`,
    );
  });
}
