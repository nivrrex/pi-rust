// 用 `createBashToolDefinition` 注册一个跑命令的工具（宿主动作帧）。
import { createBashToolDefinition } from "@earendil-works/pi-coding-agent";

export default function actionBashExtension(pi) {
  const tool = createBashToolDefinition({ description: "run a command" });
  pi.registerTool({
    name: "ext_action_bash",
    description: "通过宿主动作帧执行命令",
    parameters: { type: "object", properties: { command: { type: "string" } }, required: ["command"] },
    execute: (id, args) => tool.execute(id, args),
  });
}
