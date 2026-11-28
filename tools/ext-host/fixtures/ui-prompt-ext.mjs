// `ui_prompt_start` / `ui_prompt_end` 夹具。
export default function uiPromptProbe(pi) {
  const seen = [];
  for (const name of ["ui_prompt_start", "ui_prompt_end"]) {
    pi.on(name, (event) => {
      seen.push({ type: name, kind: event?.kind, reason: event?.reason, title: event?.title });
      return undefined;
    });
  }
  // `probe: "ui-prompt"` 是**身份标记**：同一进程里多个夹具都会设
  // `session_before_tree`，只有带这个字段的返回才说明本夹具真的接管了宿主。
  pi.on("session_before_tree", () => ({
    summary: JSON.stringify({ probe: "ui-prompt", seen }),
  }));
}
