// `registerCommand` 夹具（P9 E3）：`/record <text>` 把参数写进日志。
import fs from "node:fs";

const logPath = process.env.PI_EXT_TEST_LOG;

function record(line) {
  if (!logPath) return;
  fs.appendFileSync(logPath, `${line}\n`);
}

export default function commandExtension(pi) {
  pi.registerCommand("record", {
    description: "记录参数（测试用）",
    handler: async (args, _ctx) => {
      record(`command:record:${String(args ?? "")}`);
    },
  });
  pi.registerCommand("boom", {
    description: "总是抛错（测试错误路径）",
    handler: async () => {
      throw new Error("boom-in-command");
    },
  });
}
