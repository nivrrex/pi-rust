// 组件工厂式 `setWidget` 夹具（P9-1-E3b / S4）：只支持 `render() -> string[]`。
import fs from "node:fs";

export default function widgetFactoryExtension(pi) {
  pi.on("session_start", (_event, ctx) => {
    ctx.ui.setWidget(
      "factory",
      (_tui, theme) => ({
        render: () => [theme.fg("accent", "factory-line-1"), "factory-line-2"],
      }),
      { placement: "belowEditor" },
    );
    const log = process.env.PI_EXT_TEST_LOG;
    if (log) fs.appendFileSync(log, "widget:factory\n");
  });
}
