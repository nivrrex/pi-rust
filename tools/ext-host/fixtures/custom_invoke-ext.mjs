// `ctx.ui.custom` 在**工具调用内**打开面板（E4f）：组件收到首个按键就 done。
export default function customInvokeExtension(pi) {
  pi.registerTool({
    name: "ext_ask_inline",
    description: "工具内打开自定义面板",
    parameters: { type: "object", properties: {} },
    async execute(_id, _args, _signal, _onUpdate, ctx) {
      const value = await ctx.ui.custom((_tui, _theme, _kb, done) => ({
        render: () => ["pick-me"],
        handleInput: (data) => done(data === "\r" ? "yes" : "no"),
        invalidate() {},
      }));
      return { content: [{ type: "text", text: `answer:${String(value)}` }] };
    },
  });
}
