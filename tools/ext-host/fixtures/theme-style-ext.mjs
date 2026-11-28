// 验证 `ctx.ui.custom` 工厂拿到的 theme 的样式函数确实产出 ANSI，
// 而不是把文本原样返回（空实现会让样式静默丢失）。
export default function themeStyleProbe(pi) {
  pi.on("session_before_tree", (_event, ctx) => {
    let result = null;
    try {
      ctx.ui.custom(
        (_tui, theme) => ({
          render() {
            return [
              theme.bold("B"),
              theme.dim("D"),
              theme.italic("I"),
              theme.bg("accent", "G"),
            ].map(String);
          },
          handleInput() {},
        }),
        {},
      );
      result = "started";
    } catch (error) {
      result = String(error && error.message ? error.message : error);
    }
    return { summary: JSON.stringify({ result }) };
  });
}
