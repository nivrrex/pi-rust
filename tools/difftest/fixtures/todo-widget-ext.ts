// F36/H3：合成 todo 型扩展（与 `pi-handoff-gate` 的 Todo 面板同形：`setWidget("todo", lines,
// {placement:"belowEditor"})`）。用途：验证 pi-rs 的 widget 渲染与 TS 一致。
export default function (pi) {
  pi.on("session_start", (_e, ctx) => {
    if (typeof ctx.ui?.setWidget !== "function") return;
    ctx.ui.setWidget(
      "todo",
      ["Todo ▰▰▱▱ 2/4", " ▶ 当前项", " · 待办一项"],
      { placement: "belowEditor" },
    );
  });
}
