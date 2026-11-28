// `thinking_level_select` 夹具：记下每次载荷，用 `session_before_tree` 的返回值带回。
export default function thinkingLevelProbe(pi) {
  const seen = [];
  pi.on("thinking_level_select", (event) => {
    seen.push(event);
    return undefined;
  });
  pi.on("session_before_tree", () => ({ summary: JSON.stringify({ seen }) }));
}
