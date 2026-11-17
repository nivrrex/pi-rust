// `session_before_tree` 夹具：订阅但不介入（返回 undefined）。
export default function beforeTreeNoop(pi) {
  pi.on("session_before_tree", () => undefined);
}
