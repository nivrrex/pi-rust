// `session_info_changed` 夹具：记下每次载荷，用 `session_before_tree` 的返回值带回。
export default function sessionInfoProbe(pi) {
  const seen = [];
  pi.on("session_info_changed", (event) => {
    seen.push(event);
    return undefined;
  });
  pi.on("session_before_tree", () => ({ summary: JSON.stringify({ seen }) }));
}
