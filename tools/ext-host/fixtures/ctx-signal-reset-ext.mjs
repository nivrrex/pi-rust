// 验证 `ctx.signal` 的**生命周期**：一次 abort 不应污染后续操作。
// 第一次事件里主动 abort；后续事件构造新 ctx（宿主每次下发），
// 记下每次读到的 signal 状态。
export default function ctxSignalResetProbe(pi) {
  const seen = [];
  let first = true;
  pi.on("session_before_tree", (_event, ctx) => {
    const signal = ctx?.signal;
    const aborted = signal instanceof AbortSignal ? signal.aborted : null;
    seen.push({ aborted });
    if (first) {
      first = false;
      try {
        ctx.abort();
      } catch (error) {
        seen.push({ error: String(error && error.message ? error.message : error) });
      }
      seen.push({ afterAbort: signal instanceof AbortSignal ? signal.aborted : null });
    }
    return { summary: JSON.stringify({ seen }) };
  });
}
