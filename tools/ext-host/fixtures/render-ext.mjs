// 渲染钩子夹具（P9-1-E4a）：只用 Text/Spacer/Container；另有一个用 SelectList 的渲染器
// 用来验证"不支持 ⇒ unsupported"。
import { Container, SelectList, Spacer, Text } from "@earendil-works/pi-tui";

export default function renderExtension(pi) {
  pi.registerTool({
    name: "ext_render_ok",
    description: "渲染器只用 Text/Spacer/Container",
    parameters: { type: "object", properties: {} },
    renderCall(args, theme) {
      const box = new Container();
      box.addChild(new Text(theme.bold(`call:${JSON.stringify(args)}`), 1, 0));
      box.addChild(new Spacer(1));
      box.addChild(new Text("second", 1, 0));
      return box;
    },
    async execute() {
      return { content: [{ type: "text", text: "ok" }] };
    },
  });
  pi.registerTool({
    name: "ext_render_unsupported",
    description: "渲染器用了 SelectList",
    parameters: { type: "object", properties: {} },
    renderCall() {
      return new SelectList(["a", "b"]);
    },
    async execute() {
      return { content: [{ type: "text", text: "ok" }] };
    },
  });
}
