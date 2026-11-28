// TUI 渲染钩子夹具（P9-1-E4a / S5）：只用 Text/Spacer/Container。
import { Container, Spacer, Text } from "@earendil-works/pi-tui";

export default function renderTuiExtension(pi) {
  pi.registerTool({
    name: "ext_echo",
    description: "回显文本（渲染钩子夹具）",
    parameters: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
    renderCall(args, theme) {
      const box = new Container();
      box.addChild(new Text(theme.fg("accent", `echo-extension:${String(args?.text ?? "")}`), 1, 0));
      box.addChild(new Spacer(1));
      box.addChild(new Text(theme.fg("dim", "second-rendered-line"), 1, 0));
      return box;
    },
    renderResult(result, options, theme) {
      const box = new Container();
      box.addChild(new Text(theme.fg("success", `result-extension:${result?.content?.[0]?.text ?? ""}`), 1, 0));
      box.addChild(new Spacer(1));
      box.addChild(new Text(theme.fg("dim", `expanded=${String(options?.expanded)}`), 1, 0));
      return box;
    },
    async execute(_id, args) {
      return { content: [{ type: "text", text: `echo:${String(args?.text ?? "")}` }] };
    },
  });
}
