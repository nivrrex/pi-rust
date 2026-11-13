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

// ── 剪贴板（F57/C）───────────────────────────────────────────────────────────
//
// 与 ref `utils/clipboard.ts:74-160` + `utils/clipboard-command.ts:1-53` 逐条对齐：
//   1. 非 linux：先 `getNativeClipboard().setText`（我们没有本地实现，等价于失败继续）；
//   2. 平台命令：darwin `pbcopy` / win32 `clip` / linux：`termux-clipboard-set`（`TERMUX_VERSION`）、
//      `wl-copy`（`WAYLAND_DISPLAY`）、`xclip`/`xsel`（`DISPLAY`）；
//   3. WSL：有 `WT_SESSION` 先 OSC 52，否则 PowerShell（`wslpath` 转路径后 `Set-Clipboard`）；
//   4. OSC 52：远程会话（`SSH_CONNECTION`/`SSH_CLIENT`/`MOSH_CONNECTION`）或
//      **headless**（linux 且无 `DISPLAY`/`WAYLAND_DISPLAY`/`TERMUX_VERSION`）且前面都失败时；
//      base64 后长度 > 100_000 ⇒ 抛 `Clipboard unavailable: text exceeds the OSC 52 size limit`；
//   5. 全部失败：按平台给出 ref 的具体错误文案（缺失依赖时提示装什么）。
//
// `runClipboardCommand` 的语义照 ref：**undefined = 失败**，空 Buffer = 成功；
// 有 `input` 时 stdin 写入文本、否则 stdout 收数据；超时 `SIGKILL` 并按失败处理。
const MAX_OSC52_ENCODED_LENGTH = 100_000;

function isRemoteSession(env) {
  return Boolean(env.SSH_CONNECTION || env.SSH_CLIENT || env.MOSH_CONNECTION);
}

/** 写 OSC 52 到 stdout；超长返回 false（ref `clipboard.ts:18-24`）。 */
function emitOsc52(text, stream = process.stdout) {
  const encoded = Buffer.from(text, "utf8").toString("base64");
  if (encoded.length > MAX_OSC52_ENCODED_LENGTH) return false;
  stream.write(`\x1b]52;c;${encoded}\x07`);
  return true;
}

/**
 * 跑一条剪贴板命令（ref `clipboard-command.ts:5-53`）。
 * 返回 `undefined` 表示失败；返回 Buffer 表示成功（读命令的内容，写命令为空 Buffer）。
 */
function runClipboardCommand(command, args, options = {}) {
  return new Promise((resolve) => {
    const { spawn } = require("node:child_process");
    const hasInput = options.input !== undefined;
    const child = spawn(command, args, {
      stdio: ["pipe", hasInput ? "ignore" : "pipe", "ignore"],
      windowsHide: true,
      // `options.env` 只影响"决策用哪个命令"；命令本身的查找走 `PATH`。
      // 测试需要把 `PATH` 指向只含假命令的目录，所以这里必须把 env 透下去
      // （ref 没有这一步是因为它跑在真实环境里；这不改变生产行为——生产不传 env）。
      ...(options.env ? { env: options.env } : {}),
    });
    const chunks = [];
    let length = 0;
    let settled = false;
    const finish = (result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(result);
    };
    const abort = () => {
      try {
        child.kill("SIGKILL");
      } catch {
        // 已退出。
      }
      child.stdout?.destroy();
      child.stdin?.destroy();
      finish(undefined);
    };
    const timer = setTimeout(abort, options.timeoutMs ?? 3000);
    child.on("error", () => finish(undefined));
    child.on("close", (code) => {
      if (!settled) finish(code === 0 ? Buffer.concat(chunks, length) : undefined);
    });
    child.stdout?.on("data", (chunk) => {
      if (settled) return;
      length += chunk.length;
      if (length > (options.maxBufferBytes ?? 50 * 1024 * 1024)) abort();
      else chunks.push(chunk);
    });
    child.stdin?.on("error", () => {}); // 写命令可能先退出
    child.stdin?.end(options.input);
  });
}

/** WSL 无 WSLg 时经 PowerShell 写 Windows 剪贴板（ref `clipboard.ts:34-58`）。 */
async function copyViaWindowsClipboard(text, env) {
  const { mkdtempSync, writeFileSync, rmSync } = require("node:fs");
  const dir = mkdtempSync(path.join(os.tmpdir(), "pi-wsl-clip-"));
  const tmpFile = path.join(dir, "clip.txt");
  try {
    writeFileSync(tmpFile, text, { encoding: "utf8", mode: 0o600 });
    const listed = await runClipboardCommand("wslpath", ["-w", tmpFile], { timeoutMs: 1000, env });
    const winPath = listed?.toString("utf8").trim();
    if (!winPath) return false;
    const script = `Set-Clipboard -Value ([System.IO.File]::ReadAllText('${winPath.replaceAll("'", "''")}', [System.Text.Encoding]::UTF8))`;
    const result = await runClipboardCommand("powershell.exe", ["-NoProfile", "-Command", script], {
      timeoutMs: 5000,
      env,
    });
    return result !== undefined;
  } catch {
    return false;
  } finally {
    try {
      rmSync(dir, { recursive: true, force: true });
    } catch {
      // 可能没创建成功。
    }
  }
}

function isWsl(env) {
  if (process.platform !== "linux") return false;
  if (env.WSL_DISTRO_NAME || env.WSL_INTEROP) return true;
  try {
    return require("node:fs").readFileSync("/proc/version", "utf8").toLowerCase().includes("microsoft");
  } catch {
    return false;
  }
}

/** ref `utils/clipboard.ts:74` `copyToClipboard`。 */
export async function copyToClipboard(text, options = {}) {
  const value = String(text ?? "");
  const env = options.env ?? process.env;
  const platform = options.platform ?? process.platform;
  const stream = options.stream ?? process.stdout;
  let copied = false;
  // ① 非 linux 先试原生剪贴板（我们没有本地实现 ⇒ 直接进入命令回退）。
  if (platform !== "linux") {
    // 留出接口：未来接 X11/Wayland 原生实现时插在这里。
  }
  // ② 平台命令。
  if (!copied) {
    const commands = [];
    if (platform === "darwin") commands.push(["pbcopy", []]);
    else if (platform === "win32") commands.push(["clip", []]);
    else {
      if (env.TERMUX_VERSION) commands.push(["termux-clipboard-set", []]);
      if (env.WAYLAND_DISPLAY) commands.push(["wl-copy", []]);
      if (env.DISPLAY) commands.push(["xclip", ["-selection", "clipboard"]], ["xsel", ["--clipboard", "--input"]]);
    }
    for (const [command, args] of commands) {
      if (
        (await runClipboardCommand(command, args, { input: value, timeoutMs: 5000, env })) !== undefined
      ) {
        copied = true;
        break;
      }
    }
  }
  // ③ WSL：Windows Terminal 优先 OSC 52，否则 PowerShell。
  let osc52Emitted = false;
  if (!copied && platform === "linux" && isWsl(env)) {
    if (env.WT_SESSION) osc52Emitted = emitOsc52(value, stream);
    copied = osc52Emitted || (await copyViaWindowsClipboard(value, env));
  }
  // ④ OSC 52：远程会话或 headless 且前面都失败。
  const headless = platform === "linux" && !env.DISPLAY && !env.WAYLAND_DISPLAY && !env.TERMUX_VERSION;
  let oversized = false;
  if (!osc52Emitted && (isRemoteSession(env) || (!copied && headless))) {
    if (emitOsc52(value, stream)) copied = true;
    else oversized = true;
  }
  if (copied) return;
  if (oversized) throw new Error("Clipboard unavailable: text exceeds the OSC 52 size limit");
  if (platform === "linux") {
    if (env.TERMUX_VERSION) {
      throw new Error("Clipboard unavailable: install the Termux:API app and `termux-api` package");
    }
    if (env.WAYLAND_DISPLAY) {
      throw new Error("Clipboard unavailable: install `wl-clipboard` (`wl-copy`) or check Wayland access");
    }
    if (env.DISPLAY) {
      throw new Error("Clipboard unavailable: install `xclip` or `xsel`, or check X11 access");
    }
  }
  throw new Error("Clipboard unavailable");
}

// ref `components/assistant-message.ts:91` `updateContent(message, isStreaming)`：内容变化时重建行。
// pi-ai-summary 的 thinking-label 适配器以 `updateContent` + `render` 作为"原型可用"判据，
// 缺失就拒绝安装并弹 `[PI_AI_SUMMARY_INSTALL_FAILED] thinking adapter …`（真机 ②）。
// 我们的助手行由 pi-tui 原生渲染，该方法不会被调用 ⇒ 空实现即可。
export const AssistantMessageComponent = class extends Container {
  constructor() {
    super();
    // F51/Z1：`pi-ai-summary` 的 thinking 适配器会写这两个字段
    // （`adapters/thinking-label.ts:346` 的 `row.hiddenThinkingLabel = buildThinkingLabel(...)`、
    // `:328-341` 的 `row.hideThinkingBlock = true`）。宿主在 `ext.render.thinking` 里
    // 建一个本实例、跑扩展的 `updateContent`，再把标签读回来给 Rust 用。
    this.hideThinkingBlock = false;
    this.hiddenThinkingLabel = undefined;
  }
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
    // F51：宿主用它区分"扩展没接管"与"扩展接管后又回落到 originalRender"
    // （后者只可能是扩展内部 catch 吞掉了异常，肉眼看不出来）。
    this.__originalRenderCalled = true;
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
