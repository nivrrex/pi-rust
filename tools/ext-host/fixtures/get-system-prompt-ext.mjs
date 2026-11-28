// `ctx.getSystemPrompt()` 夹具。
export default function getSystemPromptProbe(pi) {
  let seen = null;
  pi.on("session_before_tree", (_event, ctx) => {
    try {
      seen = ctx && typeof ctx.getSystemPrompt === "function" ? ctx.getSystemPrompt() : "NO-METHOD";
    } catch (error) {
      seen = String(error && error.message ? error.message : error);
    }
    return { summary: JSON.stringify({ seen }) };
  });
}
