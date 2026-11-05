// overlay 几何探针（E4h）：3 行、宽度不同，用来反推 anchor 与列偏移。
import { Text } from "@earendil-works/pi-tui";

export default function overlayProbeExtension(pi) {
  pi.on("session_start", (_event, ctx) => {
    void ctx.ui.custom(
      (_tui, theme) => ({
        render: () => [
          theme.fg("accent", "OVL-A"),
          theme.fg("accent", "OVL-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
          theme.fg("accent", "OVL-C"),
        ],
        handleInput() {},
        invalidate() {},
      }),
      { overlay: true },
    );
  });
}
