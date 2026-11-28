// `pi.setActiveTools` 夹具：在 `before_agent_start` 里把工具集缩到只剩 `read`。
//
// 验证点：宿主接受该调用（不抛错），且**下一次请求体**的 tools 只含 `read`。
export default function setActiveToolsProbe(pi) {
  pi.on("before_agent_start", () => {
    pi.setActiveTools(["read"]);
    return undefined;
  });
}
