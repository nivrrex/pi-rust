// `pi.getAllTools()` 夹具：报出它看到的工具名。
export default function getAllToolsProbe(pi) {
  pi.on("session_before_tree", () => {
    const names = (pi.getAllTools ? pi.getAllTools() : []).map((tool) => tool?.name);
    return { summary: JSON.stringify({ names }) };
  });
}
