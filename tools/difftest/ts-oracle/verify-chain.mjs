// F25/S3b：用 ref 真实 `SessionManager.open` 加载会话文件，报告条目数、叶子与分支长度。
// 用法：node verify-chain.mjs <session.jsonl>
import fs from "node:fs";
import path from "node:path";

const TS_PKG = process.env.TS_PKG_DIR ?? (() => {
  const entry = process.env.TS_PI_ENTRY;
  if (!entry) {
    console.error("需要 TS_PKG_DIR 或 TS_PI_ENTRY");
    process.exit(2);
  }
  return path.resolve(entry, "../../..");
})();

const farm = "/tmp/pi-chain-farm/node_modules/@earendil-works";
fs.mkdirSync(farm, { recursive: true });
fs.rmSync(path.join(farm, "pi-coding-agent"), { force: true });
fs.symlinkSync(TS_PKG, path.join(farm, "pi-coding-agent"));

const { SessionManager } = await import(path.join(TS_PKG, "dist/index.js"));
const file = process.argv[2];
if (!file) {
  console.error("用法: verify-chain.mjs <session.jsonl>");
  process.exit(2);
}
const manager = SessionManager.open(file);
const entries = manager.getEntries();
const branch = manager.getBranch();
process.stdout.write(
  JSON.stringify({
    entries: entries.length,
    branch: branch.length,
    leafId: entries.length > 0 ? (entries[entries.length - 1].id ?? null) : null,
    types: entries.map((entry) => entry.type),
  }) + "\n",
);
