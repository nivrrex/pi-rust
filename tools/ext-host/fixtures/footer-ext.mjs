// `ctx.ui.setFooter` 夹具（E4d）：只接受 `render() -> string[]` 的组件。
import { Text } from "@earendil-works/pi-tui";

export default function footerExtension(pi) {
  pi.on("session_start", (_event, ctx) => {
    ctx.ui.setFooter((tui, theme, footerData) => ({
      render: () => [
        theme.fg("accent", `custom-footer providers=${footerData.getAvailableProviderCount()}`),
        "second-footer-line",
      ],
    }));
  });
}
