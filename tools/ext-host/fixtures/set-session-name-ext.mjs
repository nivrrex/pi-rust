// `pi.setSessionName` / `pi.getSessionName` 夹具。
export default function setSessionNameProbe(pi) {
  pi.on("session_before_tree", () => {
    const before = pi.getSessionName ? pi.getSessionName() : "NO-GETTER";
    try {
      pi.setSessionName("renamed-by-extension");
    } catch (error) {
      return { summary: JSON.stringify({ before, error: String(error && error.message ? error.message : error) }) };
    }
    const after = pi.getSessionName ? pi.getSessionName() : "NO-GETTER";
    return { summary: JSON.stringify({ before, after }) };
  });
}
