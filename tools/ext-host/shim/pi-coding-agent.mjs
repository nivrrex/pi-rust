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

// ref `components/assistant-message.ts:91` `updateContent(message, isStreaming)`：内容变化时重建行。
// pi-ai-summary 的 thinking-label 适配器以 `updateContent` + `render` 作为"原型可用"判据，
// 缺失就拒绝安装并弹 `[PI_AI_SUMMARY_INSTALL_FAILED] thinking adapter …`（真机 ②）。
// 我们的助手行由 pi-tui 原生渲染，该方法不会被调用 ⇒ 空实现即可。
export const AssistantMessageComponent = class extends Container {
  updateContent() {}
  render() {
    return [];
  }
};
// ref `components/tool-execution.ts:67-96` 的工具行组件。
//
// **为什么不能再是空壳**（F48/X1）：`pi-ai-summary` 这类扩展不是用 `renderCall`，
// 而是直接改 `ToolExecutionComponent.prototype.render`（见该扩展
// `src/adapters/tool-render.ts:246-252`）。空壳的 `render()` 恒返回 `[]`，补丁改的
// 是一个永远不被调用的方法 —— 对 Rust 渲染零影响（真机：TS 单行折叠、pi-rs 块状）。
// 现在把它做成**可被实例化的最小实现**：字段与 ref 同名同义，`render()` 是可被替换的
// 公开方法，宿主在 `ext.render.tool` 里构造行、调 `render(width)`。
//
// `render()` 的默认实现返回 `null`（不是 `[]`）：这是"**未被打过补丁**"的标记，
// 让宿主能明确区分"扩展接管了"与"没人接管"，从而在无人接管时走 Rust 原生渲染。
export class ToolExecutionComponent extends Container {
  constructor(toolName = "", toolCallId = "", args = undefined, _options, _definition, _ui, _cwd) {
    super();
    this.toolName = String(toolName);
    this.toolCallId = String(toolCallId);
    this.args = args;
    // ref `:74-76`：构造时 `expanded = false`、`isPartial = true`。
    this.expanded = false;
    this.isPartial = true;
    this.hideComponent = false;
    this.result = undefined;
    this.rendererState = {};
  }
  /** ref `:183-186`：流式参数更新。 */
  updateArgs(args) {
    this.args = args;
    this.updateDisplay();
  }
  /** ref `:200-211`：结果回填（`isPartial=false` 表示定稿）。 */
  updateResult(result, isPartial = false) {
    this.result = result;
    this.isPartial = isPartial;
    this.updateDisplay();
  }
  /** ref `:244-247`。 */
  setExpanded(expanded) {
    this.expanded = expanded === true;
    this.updateDisplay();
  }
  /** ref `:307`：重建显示。默认什么都不做（扩展可以在此挂缓存失效）。 */
  updateDisplay() {}
  /** 默认渲染 = `null`（未打补丁）。扩展替换本方法后由宿主调用。 */
  render() {
    return null;
  }
};
export const DynamicBorder = class extends Container {};
export { Text, Container, Spacer, SelectList };
export const ExtensionAPI = undefined;
export const ExtensionContext = undefined;
export const ExtensionCommandContext = undefined;
export const SessionEntry = undefined;
export const Theme = undefined;
