// `session_before_switch` 夹具：要求**取消**切换（`{cancelled:true}`）。
//
// 这个返回值此前同样被 runner 丢弃（通用分支只 `await handler(...)`），
// 于是 `emit_session_before_switch` 永远返回 `false`——扩展无法阻止 `/new` 或
// `/resume` 的切换。F61/38 一并修好。
export default function beforeSwitchCancel(pi) {
  pi.on("session_before_switch", () => ({ cancelled: true }));
}
