// F60/5：`ctx.abort()` / `ctx.shutdown()` / `ctx.isIdle()` / `ctx.isProjectTrusted()` /
// `ctx.hasPendingMessages()` 的可用性探针。
//
// 真实使用方是 `pi-agent-resilience`（流卡住 / 文本循环时调 `ctx.abort()`，
// `index.ts:116/227`）。这些成员此前**完全缺失** ⇒ 扩展的 `try { ctx.abort() } catch {}`
// 把 `TypeError` 吞掉 ⇒ 自动恢复静默失效（用户只看到"卡住"，没有任何错误）。
//
// 本扩展把每个成员的类型与调用结果写进 `PI_EXT_TEST_LOG`，供断言。
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

export default function abortExtension(pi) {
  pi.on("session_start", async (_event, ctx) => {
    // 1. 类型探测：这些成员必须是**函数**（缺失时是 undefined）。
    record(`typeof:abort=${typeof ctx?.abort}`);
    record(`typeof:shutdown=${typeof ctx?.shutdown}`);
    record(`typeof:isIdle=${typeof ctx?.isIdle}`);
    record(`typeof:isProjectTrusted=${typeof ctx?.isProjectTrusted}`);
    record(`typeof:hasPendingMessages=${typeof ctx?.hasPendingMessages}`);
    // 2. 无参布尔方法的返回值（radix 无关，直接记字符串）。
    record(`value:isIdle=${String(ctx?.isIdle?.())}`);
    record(`value:isProjectTrusted=${String(ctx?.isProjectTrusted?.())}`);
    record(`value:hasPendingMessages=${String(ctx?.hasPendingMessages?.())}`);
    // 3. 真正调用 abort：不得抛错（缺失时会被 try/catch 吞掉，这里把结果也记下）。
    try {
      const out = await ctx?.abort?.();
      record(`call:abort=ok:${JSON.stringify(out ?? null)}`);
    } catch (error) {
      record(`call:abort=throw:${String(error && error.message ? error.message : error)}`);
    }
  });

  pi.registerCommand("ext_abort", {
    description: "调用 ctx.abort() 并记录结果（F60/5 探针）",
    handler: async (_args, ctx) => {
      try {
        const out = await ctx?.abort?.();
        record(`cmd:abort=ok:${JSON.stringify(out ?? null)}`);
      } catch (error) {
        record(`cmd:abort=throw:${String(error && error.message ? error.message : error)}`);
      }
    },
  });
}
