// `ctx.ui.setHeader` 夹具（E4d）：只接受 `render() -> string[]` 的组件。
import { Text } from "@earendil-works/pi-tui";

export default function headerExtension(pi) {
  pi.on("session_start", (_event, ctx) => {
    ctx.ui.setHeader((_tui, theme) => ({
      render: () => [theme.fg("accent", "custom-header-line-1"), "custom-header-line-2"],
    }));
  });
}
