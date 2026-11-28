// P9-F7b/S2：用 **TS 侧真实代码**（安装包 `@earendil-works/pi-coding-agent` 0.87.1 的
// `SessionManager` / `convertToLlm`）生成 `bashExecution` 会话样本与"LLM 投影" oracle。
//
// 用法：node tools/difftest/ts-oracle/gen-bash-execution.mjs
// 依赖：/tmp/f7b/node_modules 里指向安装包的符号链接（脚本自己建，不改 ref 与安装包）。
//
// 输出（两次运行必须逐字节一致）：
//   fixtures/session/bash-execution.jsonl         —— 会话文件（SessionManager 生成的 entry 形状）
//   tools/difftest/ts-oracle/fixtures-bash-execution.json —— 每种情形的 LLM 投影 + 文本
import fs from "node:fs";
import path from "node:path";

const REPO = new URL("../../../", import.meta.url).pathname;
// 安装包目录**不硬编码**：优先 `TS_PKG_DIR`，否则由 `TS_PI_ENTRY`（`.../dist/bundle/cli.js`）推导。
function resolvePackageDir() {
  if (process.env.TS_PKG_DIR) return process.env.TS_PKG_DIR;
  const entry = process.env.TS_PI_ENTRY;
  if (!entry) {
    console.error("需要 TS_PKG_DIR 或 TS_PI_ENTRY（指向 0.87.1 安装包的 dist/bundle/cli.js）");
    process.exit(2);
  }
  return path.resolve(entry, "../../..");
}
const TS_PKG = resolvePackageDir();

// 建一个最小的 node_modules 农场，让裸导入 `@earendil-works/pi-coding-agent` 解析到安装包
// （安装包是只读的，我们只在 /tmp 里放符号链接）。
const farm = "/tmp/f7b/node_modules/@earendil-works";
fs.mkdirSync(farm, { recursive: true });
fs.rmSync(path.join(farm, "pi-coding-agent"), { force: true });
fs.symlinkSync(TS_PKG, path.join(farm, "pi-coding-agent"));

const { SessionManager, convertToLlm, CURRENT_SESSION_VERSION } = await import(
  path.join(TS_PKG, "dist/index.js")
);

const CASES = [
  { id: "ok", msg: { command: "echo hi", output: "hi\n", exitCode: 0, cancelled: false, truncated: false } },
  { id: "nonzero", msg: { command: "false", output: "", exitCode: 1, cancelled: false, truncated: false } },
  { id: "cancelled", msg: { command: "sleep 10", output: "", exitCode: undefined, cancelled: true, truncated: false } },
  {
    id: "truncated",
    msg: { command: "seq 1 9999", output: "1\n2\n", exitCode: 0, cancelled: false, truncated: true, fullOutputPath: "/tmp/full.log" },
    extra: { unknownField: { keep: true } },
  },
  { id: "excluded", msg: { command: "echo secret", output: "secret\n", exitCode: 0, cancelled: false, truncated: false, excludeFromContext: true } },
  { id: "no-exit-code", msg: { command: "kill -9 $$", output: "", exitCode: undefined, cancelled: false, truncated: false } },
];

const workDir = "/tmp/f7b/work";
const sessDir = "/tmp/f7b/sess";
fs.rmSync(sessDir, { recursive: true, force: true });
fs.mkdirSync(workDir, { recursive: true });
fs.mkdirSync(sessDir, { recursive: true });

const manager = SessionManager.create(workDir, sessDir);
const timestamp = 1730000000000;

// 前后各放一条普通消息，验证"执行记录夹在对话中间"的情形。
manager.appendMessage({ role: "user", content: [{ type: "text", text: "hello" }], timestamp });
manager.appendMessage({
  role: "assistant",
  content: [{ type: "text", text: "hi there" }],
  api: "openai-completions",
  provider: "mock",
  model: "mock-model",
  usage: { input: 1, output: 1, cacheRead: 0, cacheWrite: 0, total: 2, cost: 0 },
  stopReason: "stop",
  timestamp,
});

const projections = [];
for (const [index, item] of CASES.entries()) {
  const message = {
    role: "bashExecution",
    command: item.msg.command,
    output: item.msg.output,
    exitCode: item.msg.exitCode,
    cancelled: item.msg.cancelled,
    truncated: item.msg.truncated,
    ...(item.msg.fullOutputPath ? { fullOutputPath: item.msg.fullOutputPath } : {}),
    timestamp: timestamp + index,
    ...(item.msg.excludeFromContext ? { excludeFromContext: true } : {}),
    ...(item.extra ?? {}),
  };
  manager.appendMessage(message);
  // oracle：ref 的 `convertToLlm` 对**单条**消息的投影（空数组 = 不进上下文）。
  const llm = convertToLlm([message]);
  projections.push({ id: item.id, input: message, llm });
}

manager.appendMessage({ role: "user", content: [{ type: "text", text: "after" }], timestamp: timestamp + 99 });

// ref 的 entry 里 `id` / entry 级 `timestamp` 是随机的（uuid + 当前时间）。为了"两次生成逐字节一致"，
// 生成器把这两项**规范化**为按序号推导的稳定值（`message.timestamp` 是我们自己给的值，不动）。
// 这只影响样本的可复现性，entry 的字段集合与序列化仍由 ref 代码决定。
const entries = manager.getEntries().map((entry, index) => ({
  ...entry,
  id: `e${String(index).padStart(2, "0")}`,
  parentId: index === 0 ? null : `e${String(index - 1).padStart(2, "0")}`,
  timestamp: new Date(Date.UTC(2026, 0, 1, 0, 0, index)).toISOString(),
}));
// 会话文件的第一行是 header（ref `SessionManager.open` 会校验它，否则拒绝加载）。
// 我们没走磁盘写入路径，所以按 ref 的形状补一条**稳定**的 header（版本号取自 ref 的常量）。
const header = {
  type: "session",
  version: CURRENT_SESSION_VERSION,
  id: "p9-f7b-session",
  timestamp: new Date(Date.UTC(2026, 0, 1, 0, 0, 0)).toISOString(),
  cwd: "/tmp/pi-f7b-work",
};
const JSONL = [JSON.stringify(header), ...entries.map((entry) => JSON.stringify(entry))].join("\n") + "\n";
const jsonl = JSONL;
const outJsonl = path.join(REPO, "fixtures/session/bash-execution.jsonl");
fs.mkdirSync(path.dirname(outJsonl), { recursive: true });
fs.writeFileSync(outJsonl, jsonl);

const oraclePath = path.join(REPO, "tools/difftest/ts-oracle/fixtures-bash-execution.json");
fs.writeFileSync(
  oraclePath,
  JSON.stringify(
    { generator: "tools/difftest/ts-oracle/gen-bash-execution.mjs", tsVersion: "0.87.1", projections },
    null,
    1,
  ) + "\n",
);

console.log(`entries=${entries.length} jsonl=${outJsonl} oracle=${oraclePath}`);
