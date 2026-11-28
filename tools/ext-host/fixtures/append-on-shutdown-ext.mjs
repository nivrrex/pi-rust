// `pi.appendEntry` 夹具：**只在 `session_shutdown` 里**写入。
//
// 用来钉住"退出流程必须先把扩展动作帧处理完"这条约束（F61/36）：
// TUI 退出时会先 `emit_session_shutdown` 再 `shutdown_host`，
// 而 `--print` 此前两条都没做 ⇒ 这类扩展的条目**稳定丢失**（不是偶发）。
export default function appendOnShutdown(pi) {
  pi.on("session_shutdown", () => {
    pi.appendEntry("shutdown-note", { from: "shutdown" });
  });
}
