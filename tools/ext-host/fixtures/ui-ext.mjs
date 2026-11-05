// UI 帧夹具（P9-1-E1c / S6）：`session_start` 时发三条 notify（info/warning/error）
// 与一条 setStatus，用来对拍 TUI 的渲染位置与文案。
export default function uiExtension(pi) {
  pi.on("session_start", (_event, ctx) => {
    ctx?.ui?.notify?.("plugin ready", "info");
    ctx?.ui?.notify?.("plugin degraded", "warning");
    ctx?.ui?.notify?.("plugin failed", "error");
    ctx?.ui?.setStatus?.("zeta", "zeta-status");
    ctx?.ui?.setStatus?.("alpha", "alpha-status");
  });
}
