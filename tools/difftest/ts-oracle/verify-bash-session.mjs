// P9-F7b/S3②：用 **ref 真实代码**加载 pi-rs 写出的会话文件并打印 LLM 投影。
//
// 用法：node verify-bash-session.mjs <session.jsonl>
// 输出：JSON 数组（`convertToLlm` 对每条 message entry 的投影）。
import fs from "node:fs";
import path from "node:path";

function resolvePackageDir() {
  if (process.env.TS_PKG_DIR) return process.env.TS_PKG_DIR;
  const entry = process.env.TS_PI_ENTRY;
  if (!entry) {
    console.error("需要 TS_PKG_DIR 或 TS_PI_ENTRY");
    process.exit(2);
  }
  return path.resolve(entry, "../../..");
}
const TS_PKG = resolvePackageDir();
const farm = "/tmp/f7b/node_modules/@earendil-works";
fs.mkdirSync(farm, { recursive: true });
fs.rmSync(path.join(farm, "pi-coding-agent"), { force: true });
fs.symlinkSync(TS_PKG, path.join(farm, "pi-coding-agent"));

const { SessionManager, convertToLlm } = await import(path.join(TS_PKG, "dist/index.js"));
const file = process.argv[2];
if (!file) {
  console.error("用法: verify-bash-session.mjs <session.jsonl>");
  process.exit(2);
}
const manager = SessionManager.open(file);
const messages = manager.getEntries().flatMap((entry) =>
  entry.type === "message" && entry.message ? [entry.message] : [],
);
process.stdout.write(JSON.stringify(convertToLlm(messages)) + "\n");
