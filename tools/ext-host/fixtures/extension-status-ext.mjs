// `ctx.ui.setStatus` → footer `getExtensionStatuses()` 夹具。
export default function extensionStatusProbe(pi) {
  let seen = null;
  pi.on("session_before_tree", (_e, ctx) => {
    // 在**同一次** handler 里写入再读取：避免依赖 `session_start` 是否已派发。
    ctx.ui.setStatus("probe-key", "probe-value");
    ctx.ui.setStatus("other-key", "other-value");
    ctx.ui.setStatus("probe-key", undefined); // 删除
    if (typeof ctx.ui.setFooter !== "function") return { summary: JSON.stringify({ seen: "NO-FOOTER" }) };
    ctx.ui.setFooter((_tui, _theme, footerData) => {
      const map = footerData?.getExtensionStatuses?.() ?? new Map();
      seen = [...map.entries()];
      return { render() { return ["probe"]; }, invalidate() {} };
    });
    return { summary: JSON.stringify({ seen }) };
  });
}
