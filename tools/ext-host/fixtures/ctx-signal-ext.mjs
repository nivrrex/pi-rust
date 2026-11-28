// `ctx.signal` 夹具：确认它是 `AbortSignal` 实例、且 `ctx.abort()` 会 abort 它。
export default function ctxSignalProbe(pi) {
  const seen = [];
  pi.on("session_before_tree", (_event, ctx) => {
    const signal = ctx?.signal;
    const isSignal = typeof AbortSignal !== "undefined" && signal instanceof AbortSignal;
    seen.push({ isSignal, abortedBefore: isSignal ? signal.aborted : null });
    if (isSignal) {
      signal.addEventListener("abort", () => seen.push({ event: "aborted" }), { once: true });
    }
    try {
      ctx.abort();
    } catch (error) {
      seen.push({ error: String(error && error.message ? error.message : error) });
    }
    seen.push({ abortedAfter: isSignal ? signal.aborted : null });
    return { summary: JSON.stringify({ seen }) };
  });
}
