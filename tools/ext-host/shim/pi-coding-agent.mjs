// `@earendil-works/pi-coding-agent` 的最小面（P9-1-B）。
//
// **数据型导出必须给真实值**：`getAgentDir()` 在 `pi-secrets` 的工厂期就被交给 `path.join`，
// 返回 Proxy/undefined 会直接抛 `ERR_INVALID_ARG_TYPE`（S6 实测）。
import os from "node:os";
import path from "node:path";
import { Text, Container, Spacer, SelectList } from "./pi-tui.mjs";

export function getAgentDir() {
  return process.env.PI_CODING_AGENT_DIR ?? path.join(os.homedir(), ".pi", "agent");
}

export function getPackageDir() {
  return process.env.PI_PACKAGE_DIR ?? path.dirname(process.execPath);
}

export function defineTool(tool) {
  return tool;
}

export function defineExtension(factory) {
  return factory;
}

export function createBashToolDefinition(options = {}) {
  return {
    name: "bash",
    label: "Bash",
    description: options.description ?? "Run a shell command",
    parameters: { type: "object", properties: { command: { type: "string" } }, required: ["command"] },
    // 执行由**宿主**完成（宿主动作帧 `host.action.call`），ref 的 `createBashToolDefinition`
    // 也是走宿主的 shell 执行器。
    async execute(idOrArgs, maybeArgs) {
      // ref 的签名是 `execute(toolCallId, params)`；宿主 runner 里也可能只传 params。
      const args = maybeArgs ?? idOrArgs ?? {};
      const call = globalThis.__piHostCall;
      if (typeof call !== "function") {
        throw new Error("pi-ext-host shim: 宿主动作通道不可用");
      }
      return call("bash", { command: String(args?.command ?? "") });
    },
  };
}

export function createLocalBashOperations() {
  return {
    async exec(command) {
      const call = globalThis.__piHostCall;
      if (typeof call !== "function") {
        throw new Error("pi-ext-host shim: 宿主动作通道不可用");
      }
      const result = await call("bash", { command: String(command ?? "") });
      return { output: result?.content?.[0]?.text ?? "", exitCode: result?.isError ? 1 : 0 };
    },
  };
}

export async function copyToClipboard(text) {
  throw new Error(`pi-ext-host shim: copyToClipboard 未实现（P9 后续批次）：${String(text).length} 字节`);
}

export const AssistantMessageComponent = class extends Container {};
// ref `components/tool-execution.ts`：工具行组件，`render()` + `updateDisplay()`（后者在
// 参数/结果变化时重建行）。pi-ai-summary 的渲染适配器靠这两个方法判断"原型可用"；
// 少了 `updateDisplay` 它会拒绝安装并弹 `[PI_AI_SUMMARY_INSTALL_FAILED] …incompatible prototype`
// （F28/S4 真机现象）。我们的工具行由 pi-tui 原生渲染，这两个方法不会被调用 ⇒ 空实现即可。
export const ToolExecutionComponent = class extends Container {
  updateDisplay() {}
  render() {
    return [];
  }
};
export const DynamicBorder = class extends Container {};
export { Text, Container, Spacer, SelectList };
export const ExtensionAPI = undefined;
export const ExtensionContext = undefined;
export const ExtensionCommandContext = undefined;
export const SessionEntry = undefined;
export const Theme = undefined;
