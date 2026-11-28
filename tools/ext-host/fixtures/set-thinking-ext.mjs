// `pi.setThinkingLevel` / `pi.getThinkingLevel` 夹具。
export default function setThinkingProbe(pi) {
  const seen = [];
  pi.on("session_before_tree", () => {
    const before = pi.getThinkingLevel ? pi.getThinkingLevel() : "NO-GETTER";
    try {
      pi.setThinkingLevel("high");
      seen.push("set-ok");
    } catch (error) {
      seen.push(String(error && error.message ? error.message : error));
    }
    const after = pi.getThinkingLevel ? pi.getThinkingLevel() : "NO-GETTER";
    return { summary: JSON.stringify({ before, after, seen }) };
  });
}
