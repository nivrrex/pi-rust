// `pi.getCommands()` 夹具：报出 name/source 组合。
export default function getCommandsProbe(pi) {
  pi.on("session_before_tree", () => {
    const list = (pi.getCommands ? pi.getCommands() : []).map((c) => `${c?.source}:${c?.name}`);
    return { summary: JSON.stringify({ list }) };
  });
}
