// 未知色名必须**抛错**（ref `theme.ts:323-327`），宿主侧应回落为 unsupported。
import { Text } from "@earendil-works/pi-tui";

export default function renderUnknownColorExtension(pi) {
  pi.registerTool({
    name: "ext_render_bad_color",
    description: "renderCall 用了不存在的色名",
    parameters: { type: "object", properties: {} },
    renderCall(_args, theme) {
      return new Text(theme.fg("no-such-color", "x"));
    },
    async execute() {
      return { content: [{ type: "text", text: "ok" }] };
    },
  });
}
