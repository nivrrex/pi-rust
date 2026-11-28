// F60/17：`ctx.sessionManager` 的只读 API 面探针（ref `session-manager.ts:1140-1160`）。
//
// 真实使用方 `pi-ai-summary`（用户已装）的页脚第一行：
// `ctx.sessionManager?.getCwd?.() ?? ctx.cwd ?? ""`（`src/footer-cny.ts:280`）。
// 此前没有 `getCwd` ⇒ 走 `??` 回退，多数情况下等价；但两者语义不同
// （`ctx.cwd` 是进程 cwd，会话的 `cwd` 来自会话头），补齐 API 面才不会在
// "会话 cwd ≠ 进程 cwd"时显示错目录。
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

export default function sessionManagerProbe(pi) {
  pi.on("session_start", (_event, ctx) => {
    const sm = ctx?.sessionManager;
    for (const name of [
      "getCwd",
      "getSessionDir",
      "getSessionId",
      "getSessionFile",
      "getBranch",
      "getEntries",
      "getLeafId",
    ]) {
      record(`typeof:${name}=${typeof sm?.[name]}`);
    }
    record(`value:getCwd=${typeof sm?.getCwd === "function" ? sm.getCwd() : "<none>"}`);
  });
}
