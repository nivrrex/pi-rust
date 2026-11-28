// 验证 CLI flag → `pi.getFlag` 的往返（含注册默认值回落）。
export default function flagRoundtripProbe(pi) {
  pi.registerFlag("probe-number", { description: "probe", type: "number", default: 7 });
  pi.registerFlag("probe-absent", { description: "probe", type: "number", default: 42 });
  pi.on("session_before_tree", () => {
    return {
      summary: JSON.stringify({
        provided: pi.getFlag("probe-number"),
        absent: pi.getFlag("probe-absent"),
      }),
    };
  });
}
