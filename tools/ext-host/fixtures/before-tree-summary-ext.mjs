// `session_before_tree` 夹具：**直接给出摘要**（`{summary}`）。
export default function beforeTreeSummary(pi) {
  pi.on("session_before_tree", () => ({ summary: "EXTENSION-SUMMARY" }));
}
