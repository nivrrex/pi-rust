// pi-rs 扩展宿主：Node 侧 runner（P9-1-B 第一批）。
//
// 与宿主的契约见 notes/p9-1/protocol.md（S5 决策：loopback TCP + 一次性令牌）。
// 本批只做：握手 → 加载扩展（.mjs/.js）→ 收集 registerTool → 服务 ext.tool.invoke。
// 事件分发 / UI 帧 / 渲染钩子留给后续批次（收到未知方法回 -32601）。

import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { EventEmitter } from "node:events";
import { register, createRequire } from "node:module";
import { pathToFileURL } from "node:url";

import * as shimTui from "./shim/pi-tui.mjs";
import * as shimCodingAgent from "./shim/pi-coding-agent.mjs";
import * as shimAi from "./shim/pi-ai.mjs";
import * as shimTypebox from "./shim/typebox.mjs";
import * as shimTheme from "./shim/theme-colors.mjs";

// 解析钩子必须在任何扩展 import 之前注册。
register(pathToFileURL(path.join(import.meta.dirname, "resolve.mjs")).href);

const endpoint = process.env.PI_EXT_ENDPOINT;
const token = process.env.PI_EXT_TOKEN;
const protocolVersion = Number(process.env.PI_EXT_PROTOCOL ?? "1");
const extensionPaths = JSON.parse(process.env.PI_EXT_EXTENSIONS ?? "[]");

if (!endpoint || !token) {
  console.error("[ext-host] missing PI_EXT_ENDPOINT / PI_EXT_TOKEN");
  process.exit(2);
}

const [host, portText] = endpoint.split(":");
let nextId = 0;
const pending = new Map();
const tools = new Map();
// 扩展开命令：`{ name, description, scope }`（ref `types.ts:1326-1332` 的 `RegisteredCommand`）。
// 宿主据此在前缀菜单里显示 `[<tag>] <description>`（`interactive-mode.ts:631-661`）。
const commands = [];
// 命令名 → handler（`registerCommand(name, {handler})`，ref `types.ts` 的 ExtensionCommand）。
const commandHandlers = new Map();
// 反射 ref `resource-loader.ts:807-846` 的 scope 判定：
//   <agentDir>/extensions/...   → user（tag `u`）
//   <cwd>/.pi/extensions/...    → project（tag `p`）
//   其它                          → temporary（tag `t`）
const agentDir = process.env.PI_CODING_AGENT_DIR ?? "";
const agentExtRoot = agentDir ? path.join(path.resolve(agentDir), "extensions") : "";
const projectExtRoot = path.join(process.cwd(), ".pi", "extensions");
function scopeTagForPath(extensionPath) {
  const resolved = path.resolve(extensionPath);
  const under = (root) => Boolean(root) && (resolved === root || resolved.startsWith(`${root}${path.sep}`));
  if (under(agentExtRoot)) return "u";
  if (under(projectExtRoot)) return "p";
  return "t";
}
// 当前正在执行工厂的扩展路径（`registerCommand` 无路径参数，靠它定位来源）。
let currentExtensionPath = "";

// F38/K4：渲染钩子按需重渲染。
// ref 的 `ctx.ui.setFooter(factory)` / `setWidget(key, factory)` 是**每帧调用组件 `render(width)`**
// （`components/footer.ts`、`interactive-mode.ts:2394-2422`）。此前我们只在注册时渲染一次，
// 宽度/用量/展开态变化都不刷新。这里保留工厂并按宿主请求重渲染。
let footerFactory = null;
// `ctx.ui.setStatus` 写入、footer 的 `getExtensionStatuses()` 读取（ref 同一张表）。
const extensionStatuses = new Map();
const widgetFactories = new Map();
// 渲染结果缓存：`<kind>|<key>|<width>|<digest>` → lines。digest 由宿主给出（见 tui.rs 的 state_digest）。
const renderCache = new Map();
const RENDER_CACHE_LIMIT = 64;

function cacheKey(kind, key, width, digest) {
  return `${kind}|${key}|${width}|${digest}`;
}

function rememberRender(key, lines) {
  if (renderCache.size >= RENDER_CACHE_LIMIT) {
    const oldest = renderCache.keys().next();
    if (!oldest.done) renderCache.delete(oldest.value);
  }
  renderCache.set(key, lines);
}
// `registerFlag` 注册的 flag（名 → {type, default}）与宿主解析好的实际值。
const flagSpecs = new Map();
let flagValues = {};
const events = [];
// 事件名 → 处理器数组（`pi.on` 收集；`unsubscribe` 从数组里摘除）。
const handlers = new Map();

// 宿主在每帧里带 ctx 快照（`types.ts:313-353` 的子集）；这里缓存最近一次。
// F51/Z1(e)：初始 ctx 由宿主经环境变量给（`session_start` 早于第一帧 `host.ctx`）。
let hostCtx = (() => {
  const raw = process.env.PI_EXT_CTX;
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
})();
const socket = net.connect({ host, port: Number(portText) });
socket.setNoDelay(true);
socket.setEncoding("utf8");

let buffer = "";
socket.on("data", (chunk) => {
  buffer += chunk;
  let index;
  while ((index = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, index);
    buffer = buffer.slice(index + 1);
    if (!line.trim()) continue;
    let frame;
    try {
      frame = JSON.parse(line);
    } catch {
      // 不是帧（宿主侧污染/半行）：忽略。
      continue;
    }
    handleFrame(frame);
  }
});

socket.on("close", () => {
  // 宿主断开 ⇒ 退出（父进程已死）。
  process.exit(0);
});

// F36/H6a：宿主异常断开（RST/EPIPE）时**静默退出 0**。此前没有 `error` 监听 ⇒ Node 把
// `ECONNRESET` 当未捕获异常打印整段堆栈并非零退出（真机"退出时报一堆 Node 错误"）。
socket.on("error", (error) => {
  const code = error && error.code;
  if (code === "ECONNRESET" || code === "EPIPE" || code === "ERR_STREAM_DESTROYED") {
    process.exit(0);
  }
  process.exit(0);
});

// stdin EOF = 父进程已死（宿主不监听时也能兜底退出）。
process.stdin.on("end", () => process.exit(0));
process.stdin.resume();

function send(method, params) {
  nextId += 1;
  return new Promise((resolve, reject) => {
    const id = nextId;
    pending.set(id, { resolve, reject });
    try {
    socket.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  } catch {
    // 宿主已断开：静默放弃这次请求（进程马上由 `error`/`close` 处理器退出）。
    pending.delete(id);
  }
  });
}

function respond(id, result, error) {
  const frame = error
    ? { jsonrpc: "2.0", id, error }
    : { jsonrpc: "2.0", id, result };
  socket.write(JSON.stringify(frame) + "\n");
}

function handleFrame(frame) {
  if (typeof frame.id === "number" && (frame.result !== undefined || frame.error !== undefined)) {
    const waiter = pending.get(frame.id);
    if (!waiter) return;
    pending.delete(frame.id);
    if (frame.error) waiter.reject(new Error(frame.error.message ?? "host error"));
    else waiter.resolve(frame.result);
    return;
  }
  if (typeof frame.method !== "string") return;
  void dispatch(frame);
}

// 扩展可见的 ctx（只读子集 + ui.* 单向帧）。
//
// **可变字段必须是 getter**：ref 里 `ctx.model` / `ctx.thinkingLevel` 是**活的**
// （`types.ts:325-334`：每次都读会话当前值），扩展常在闭包里长期持有 `ctx`
// （例如 `setFooter((tui, theme) => ({ render() { … ctx.thinkingLevel … } }))`）。
// 此前这里返回的是一个**快照对象** ⇒ `session_start` 之后宿主再更新模型/档位
// 都传不进那个闭包，页脚永远显示 `thinking off`（真机 G2，实测）。
// 一个对象一次性给全，属性用 getter 保持实时。
// `ctx.signal` 背后的控制器：**跟随当前操作**重建。
//
// ref `extensions/types.ts:340` 的 `signal` 是 `get signal() { return getSignalFn() }`
// （`runner.ts:858-861`）→ `() => this.agent.signal`（`agent-session.ts:3100`），
// 即**当前这次操作**的信号，abort 一次不会污染后续操作。
//
// 因此不能在进程级固定一个控制器：那样一旦 abort，之后所有回合扩展读到的
// signal 都是 aborted，`pi-agent-resilience` 的"外部已取消就别重试"保护会永久生效。
//
// **触发重建的时机**：事件边界。宿主**不会**在每次事件里附带 ctx（`emit_value`
// 只带 payload；ctx 经 `PI_EXT_CTX` 在握手时下发一次，之后 `set_ctx` 只更新宿主
// 内部状态），所以不能靠"收到新 ctx"判断。这里在每次构造 `ctx`（= 一次新操作开始）
// 时检查：若当前控制器已 abort，换一个新的——语义上等价于 ref 的"每次操作新 signal"。
let abortController = new AbortController();

function resetAbortSignalIfAborted() {
  if (abortController.signal.aborted) {
    abortController = new AbortController();
  }
}

// 宿主下发 ctx（握手一次）：只更新快照。
function applyHostCtx(next) {
  if (!next) return;
  hostCtx = next;
}

function contextForExtension() {
  // 每次构造 ctx = 一次新操作开始 ⇒ 若上次已 abort，换新 signal。
  resetAbortSignalIfAborted();
  const ctx = {
    mode: hostCtx.mode,
    hasUI: hostCtx.hasUI,
    // ref `extensions/types.ts:340`：`signal: AbortSignal | undefined`。
    // `pi-agent-resilience` 用 `instanceof AbortSignal` 判断（`abortSignal()`），
    // 之前没有该字段 ⇒ 返回 undefined、那层取消保护静默失效。
    // getter：与 ref 的 `get signal()` 一致，每次读都取**当前**控制器。
    get signal() {
      return abortController.signal;
    },
    // ref `types.ts:351-352` 的 `ctx.getSystemPrompt()`：当前生效的完整系统提示。
    // 值来自宿主下发的 ctx 快照（`set_ctx_system_prompt`）。
    getSystemPrompt() {
      return hostCtx.systemPrompt ?? "";
    },
    // ref `types.ts:1854` 的 `ctx.sessionManager`（只读子集）：宿主只给会话文件路径，
    // 条目由这里按需读文件解析（文件即事实来源，不做缓存）。
    sessionManager: {
      getSessionId() {
        return typeof hostCtx.sessionId === "string" && hostCtx.sessionId.length > 0
          ? hostCtx.sessionId
          : undefined;
      },
      getSessionFile() {
        return hostCtx.sessionFile ?? undefined;
      },
      // ref `session-manager.ts:1467` 的 `getBranch(fromId?)`：沿 `parentId` 从叶子回溯。
      getBranch(fromId) {
        const entries = this.getEntries().filter((entry) => entry?.type !== "session");
        if (entries.length === 0) return [];
        const byId = new Map();
        for (const entry of entries) byId.set(entry.id, entry);
        // 默认起点是**当前叶子**（ref `:1469` 的 `fromId ?? this.leafId`）。
        let cursor = fromId ?? this.getLeafId() ?? entries[entries.length - 1]?.id;
        const branch = [];
        let guard = 0;
        while (cursor !== undefined && cursor !== null && guard < entries.length + 1) {
          const entry = byId.get(cursor);
          if (!entry) break;
          branch.push(entry);
          cursor = entry.parentId;
          guard += 1;
        }
        return branch.reverse();
      },
      // ref `session-manager.ts:1469`：`this.leafId` —— **当前叶子**，不是
      // "文件最后一条"。分叉后导航到中间节点时两者不同，用后者会算错分支。
      getLeafId() {
        if (typeof hostCtx.leafId === "string" && hostCtx.leafId.length > 0) return hostCtx.leafId;
        const entries = this.getEntries().filter((entry) => entry?.type !== "session");
        return entries[entries.length - 1]?.id ?? null;
      },
      getSessionDir() {
        const file = hostCtx.sessionFile;
        return file ? path.dirname(file) : undefined;
      },
      // ref `session-manager.ts:1140-1142`：`getCwd()` 返回**会话的工作目录**。
      //
      // 真实使用方 `pi-ai-summary`（用户已装）的页脚第一行：
      // `ctx.sessionManager?.getCwd?.() ?? ctx.cwd ?? ""`（`src/footer-cny.ts:280`）。
      // 此前没有这个成员 ⇒ 走 `??` 回退到 `ctx.cwd`，多数情况下等价，
      // 但两者语义不同：`ctx.cwd` 是**进程 cwd**，而会话的 `cwd` 来自会话头
      // （`--session <file>` / `-c` 恢复时可能不同于进程 cwd，ref 的页脚显示的是会话的）。
      getCwd() {
        if (typeof hostCtx.cwd === "string" && hostCtx.cwd.length > 0) return hostCtx.cwd;
        return process.cwd();
      },
      getEntries() {
        const file = hostCtx.sessionFile;
        if (!file) return [];
        try {
          // F51/Z1(e)：**坏行只跳过**，不要让整份会话退回空数组。
          // 此前 `.map(JSON.parse)` 一遇坏行就抛 ⇒ `catch` 返回 `[]` ⇒ 扩展页脚算出的
          // 用量/费用全为 0（真机：TS 显示 `¥954.32`，pi-rs 只有 `144.6%/197k (auto)`）。
          const out = [];
          for (const line of fs.readFileSync(file, "utf8").split("\n")) {
            if (line.trim().length === 0) continue;
            try {
              out.push(JSON.parse(line));
            } catch {
              // 跳过坏行（ref `session-manager.ts:616-624` 同样跳过解析失败的行）。
            }
          }
          return out;
        } catch {
          return [];
        }
      },
    },
    // ref `types.ts:294-300` 的 `ContextUsage`；宿主在每帧的 ctx 里带过来。
    getContextUsage() {
      return hostCtx.usage ?? undefined;
    },
    // ref `types.ts:342` 的 `ctx.abort()`：中止当前 agent 操作。
    //
    // 真实使用方是 `pi-agent-resilience`（流卡住 `stream_stall` / 文本循环检测，
    // `index.ts:116/227`）。此前这里**没有**这个成员 ⇒ 扩展的
    // `try { ctx.abort() } catch {}` 把 `TypeError: ctx.abort is not a function`
    // 吞掉 ⇒ 自动恢复静默失效（既不中止也不报错，用户只能看到"卡住"）。
    //
    // 走 `host.action.call`（与 `createBashToolDefinition` 同一条通道）：宿主侧
    // 发一条 `TuiEvent::AbortTurn`，由主循环取消该回合的 `CancellationToken`。
    abort() {
      // 先本地触发 signal（同步语义、立即生效），再通知宿主真正中止。
      abortController.abort();
      return send("host.action.call", { action: "abort", args: {} });
    },
    // ref `types.ts:336/338/344/346`：这四项都属"阅读会话运行时状态"。
    // 未实现时**显式返回保守值**而不是 undefined（扩展里的 `ctx.isIdle?.()` 之类
    // 若拿到 undefined 会当成"未知"，而 ref 语义是布尔/无参方法）。
    // - `isIdle`：宿主在每轮 `agent_settled` 之后更新 `hostCtx.isIdle`；
    //   没有该字段时按"未知即不空闲"处理（宁可让扩展多等，也不误导它提前动作）。
    isIdle() {
      return hostCtx.isIdle === true;
    },
    // - `isProjectTrusted`：宿主在启动时按 `<agentDir>/trust.json` 算出当前 cwd 的
    //   决定，经 `PI_EXT_CTX.isProjectTrusted` 传进来（F61/21 起有 `/trust` 状态机）。
    //   缺字段时保守返回 false（= 未信任），与 ref 在没有信任决定时的取值一致。
    isProjectTrusted() {
      return hostCtx.isProjectTrusted === true;
    },
    // - `hasPendingMessages`：队列在宿主侧，扩展看不到 ⇒ 无待发消息。
    hasPendingMessages() {
      return false;
    },
    // - `shutdown`：让宿主退出。走同一条动作通道，由主循环决定退出方式
    //   （TUI 走正常退出路径，会先派发 `session_shutdown`）。
    shutdown() {
      return send("host.action.call", { action: "shutdown", args: {} });
    },
    ui: {
      // ref `types.ts:276-282`：`ctx.ui.theme` 是**当前主题对象**（扩展用它上色，
      // 例如 `pi-handoff-gate` 的 Todo 面板：`ctx.ui.theme.fg(role, text)`）。
      // 缺失时扩展抛 `Cannot read properties of undefined`，被自身的 try/catch 吞掉 ⇒
      // 面板静默不显示（F39/M0 实测：TS 有 Todo 面板、pi-rs 完全没有）。
      theme: makeThemeStub(),
      // ref `types.ts:284` / `interactive-mode.ts:2585`：工具输出是否展开。
      getToolsExpanded: () => hostCtx.toolsExpanded === true,
      setToolsExpanded: (expanded) =>
        send("host.ui.setToolsExpanded", { expanded: expanded === true }),
      // 第二个参数是级别（ref `types.ts:148` 的 `"info" | "warning" | "error"`）。
      notify: (message, type) =>
        send("host.ui.notify", {
          message: String(message ?? ""),
          ...(type ? { type: String(type) } : {}),
        }),
      // ref `types.ts:168-190` 的 `setFooter` 注释：footer 的
      // `getExtensionStatuses()` 读的就是 `setStatus` 写入的表。因此这里**本地维护**
      // （同步可读），同时通知宿主（宿主侧只用于诊断记录）。
      setStatus: (key, text) => {
        const name = String(key);
        if (text === undefined || text === null) extensionStatuses.delete(name);
        else extensionStatuses.set(name, String(text));
        return send("host.ui.setStatus", { key: name, text: text === undefined ? null : String(text) });
      },
      // ref `types.ts:176-185`：`string[]` 形态可直接下发；组件工厂形态仍然抛错。
      setWidget: (key, content, options) => {
        if (typeof content === "function") {
          // 组件工厂：调用一次工厂，**只接受** `render()` 返回字符串数组的组件
          // （pi-remote-control 的 QR 就是这种形态）；其它形态显式报错，不静默降级。
          widgetFactories.set(String(key ?? ""), content);
          const themeStub = makeThemeStub();
          const component = content({ requestRender() {} }, themeStub);
          // 宽度：ref 每帧用终端宽度渲染；注册时先按当前宽度渲染一次。
          const rendered = component?.render?.(Number(hostCtx.width ?? 80));
          if (!Array.isArray(rendered)) {
            throw new Error("pi-ext-host: ctx.ui.setWidget 工厂返回的组件不支持 render() -> string[]");
          }
          return send("host.ui.setWidget", {
            key: String(key ?? ""),
            lines: rendered.map((line) => String(line)),
            fromLines: false,
            ...(options?.placement ? { placement: String(options.placement) } : {}),
          });
        }
        if (typeof content === "function") widgetFactories.set(String(key ?? ""), content);
        else widgetFactories.delete(String(key ?? ""));
        return send("host.ui.setWidget", {
          key: String(key ?? ""),
          lines: Array.isArray(content) ? content.map((line) => String(line)) : null,
          fromLines: Array.isArray(content),
          ...(options?.placement ? { placement: String(options.placement) } : {}),
        });
      },
      // ref `types.ts:189-193`：`setFooter(factory | undefined)`。工厂只接受
      // `render() -> string[]` 的组件（与 setWidget 同一降级口径）；`undefined` 恢复内置页脚。
      setFooter: (factory) => {
        if (factory === undefined) {
          footerFactory = null;
          return send("host.ui.setFooter", { lines: null });
        }
        if (typeof factory !== "function") {
          throw new Error("pi-ext-host: ctx.ui.setFooter 只接受工厂函数或 undefined");
        }
        footerFactory = factory;
        let component;
        try {
          component = factory({ requestRender() {} }, makeThemeStub(), footerDataStub());
        } catch (error) {
          const logPath = process.env.PI_TOOL_ROW_LOG;
          if (logPath) {
            try {
              fs.appendFileSync(logPath, `setFooter factory threw: ${error?.message ?? error}\n`);
            } catch {}
          }
          throw error;
        }
        // 宽度：ref 的页脚每帧用**终端宽度**渲染（`footer.ts` 的 `render(width)`）。
        // 此前这里不传宽度 ⇒ 扩展里的 `truncateToWidth(x, undefined)` 把 cwd 截成 `...`、
        // 模型名整段丢失（F36/H2 实测）。
        const rendered = component?.render?.(Number(hostCtx.width ?? 80));
        if (!Array.isArray(rendered)) {
          throw new Error("pi-ext-host: ctx.ui.setFooter 工厂返回的组件不支持 render() -> string[]");
        }
        return send("host.ui.setFooter", { lines: rendered.map((line) => String(line)) });
      },
      // ref `types.ts:201-215`：`custom(factory, {overlay, overlayOptions, onHandle})`。
      // 只支持"挂到编辑器位置、组件实现 render/handleInput"这一最小集合；
      // 按键由宿主以 `ext.custom.input` 请求下发（见 protocol.md 的 E4e 段）。
      custom: async (factory, options) => {
        // overlay/overlayOptions/onHandle：本实现没有 overlay 层，按"挂编辑器位置"渲染并记录警告
        // （ref 会浮在内容之上；文本内容一致，位置不同）。不抛错，否则 request_secret 之类
        // 依赖 overlay 的扩展会直接失败。
        // overlay 由宿主合成在主屏之上（ref `interactive-mode.ts:2902-2917`）；onHandle 未实现。
        const overlay = Boolean(options?.overlay);
        if (options?.onHandle) {
          process.stderr.write("[ext-host] ctx.ui.custom: onHandle 未实现\n");
        }
        if (typeof factory !== "function") {
          throw new Error("pi-ext-host: ctx.ui.custom 需要一个工厂函数");
        }
        const t0 = Date.now();
        const mark = (label) => {
          if (process.env.PI_EXT_DEBUG_INVOKE) {
            process.stderr.write(`[dbg] custom ${label} +${Date.now() - t0}ms\n`);
          }
        };
        mark("start");
        let closed = false;
        let result;
        let resolveDone;
        const finished = new Promise((resolve) => {
          resolveDone = resolve;
        });
        const done = (value) => {
          if (closed) return;
          closed = true;
          result = value;
          resolveDone();
        };
        const component = await Promise.resolve(
          factory({ requestRender() {} }, makeThemeStub(), keybindingsStub(), done),
        );
        if (!component || typeof component.render !== "function") {
          throw new Error("pi-ext-host: ctx.ui.custom 的组件必须实现 render(width)");
        }
        activeCustom = {
          component,
          isClosed: () => closed,
          result: () => result,
        };
        await send("host.ui.custom.open", {
          lines: component.render(80).map((line) => String(line)),
          overlay,
        });
        mark("opened");
        if (!closed) await finished;
        mark("finished");
        activeCustom = null;
        try {
          component.dispose?.();
        } catch {
          /* dispose 的异常不影响结果 */
        }
        mark("return");
        return result;
      },
      // ref `types.ts:199`：`setTitle(title)` —— 设置终端窗口/标签标题（OSC 2）。
      setTitle: (title) => send("host.ui.setTitle", { title: String(title ?? "") }),
      // ref `types.ts:196`：`setHeader(factory | undefined)`，工厂只接受 `render() -> string[]`。
      setHeader: (factory) => {
        if (factory === undefined) return send("host.ui.setHeader", { lines: null });
        if (typeof factory !== "function") {
          throw new Error("pi-ext-host: ctx.ui.setHeader 只接受工厂函数或 undefined");
        }
        const component = factory({ requestRender() {} }, makeThemeStub());
        const rendered = component?.render?.();
        if (!Array.isArray(rendered)) {
          throw new Error("pi-ext-host: ctx.ui.setHeader 工厂返回的组件不支持 render() -> string[]");
        }
        return send("host.ui.setHeader", { lines: rendered.map((line) => String(line)) });
      },
      // 无 UI（print/rpc/headless）：ref 的 `noOpUIContext`（`runner.ts:320-325`）
      // 直接给"取消"语义 —— `select`/`input` → undefined、`confirm` → false。
      // 有 UI：通过 `host.ui.*` 询问宿主（宿主再交给前端面板，ref `interactive-mode.ts:2530-2545`）。
      select: (title, options) => {
        if (!hostCtx.hasUI) return Promise.resolve(undefined);
        return send("host.ui.select", {
          title: String(title ?? ""),
          options: Array.isArray(options) ? options.map((item) => String(item)) : [],
        });
      },
      confirm: async (title) => {
        if (!hostCtx.hasUI) return false;
        const answer = await send("host.ui.confirm", { title: String(title ?? "") });
        return Boolean(answer);
      },
      input: (title, placeholder) => {
        if (!hostCtx.hasUI) return Promise.resolve(undefined);
        return send("host.ui.input", {
          title: String(title ?? ""),
          ...(placeholder ? { placeholder: String(placeholder) } : {}),
        });
      },
    },
  };
  // **实时字段**（见函数头注释）：模型 / 思考档位 / 宽度 / cwd 必须每次读取时取当前值，
  // 否则长期持有 `ctx` 的扩展（页脚工厂、闭包里的 handler）会永远看到注册那一刻的快照。
  Object.defineProperties(ctx, {
    cwd: { enumerable: true, get: () => hostCtx.cwd },
    model: { enumerable: true, get: () => hostCtx.model },
    thinkingLevel: { enumerable: true, get: () => hostCtx.thinkingLevel },
    width: {
      enumerable: true,
      get: () => (typeof hostCtx.width === "number" ? hostCtx.width : 80),
    },
  });
  return ctx;
}

async function dispatch(frame) {
  const { id, method, params } = frame;
  applyHostCtx(params?.ctx);
  if (method === "ext.tool.invoke") {
    const tool = tools.get(params?.toolName);
    if (!tool) {
      respond(id, undefined, { code: -32601, message: `unknown tool ${params?.toolName}` });
      return;
    }
    try {
      applyHostCtx(params?.ctx);
      if (process.env.PI_EXT_DEBUG_INVOKE) process.stderr.write(`[dbg] execute start ${params?.toolName}\n`);
      const result = await tool.execute(
        `ext-${id}`,
        params.args ?? {},
        undefined,
        undefined,
        contextForExtension(),
      );
      if (process.env.PI_EXT_DEBUG_INVOKE) process.stderr.write(`[dbg] execute done ${params?.toolName}\n`);
      respond(id, normalizeResult(result));
    } catch (error) {
      respond(id, {
        content: [{ type: "text", text: String(error && error.message ? error.message : error) }],
        isError: true,
      });
    }
    return;
  }
  if (method === "ext.custom.input") {
    if (!activeCustom) {
      respond(id, undefined, { code: -32601, message: "no active custom component" });
      return;
    }
    try {
      if (process.env.PI_EXT_DEBUG_INVOKE) {
        // **只记长度与类型**：按键内容可能是密钥明文（request_secret 的掩码输入逐字符下发）。
        const key = String(params?.key ?? "");
        process.stderr.write(`[dbg] custom input len=${key.length} t=${Date.now()}\n`);
      }
      activeCustom.component.handleInput(String(params?.key ?? ""));
      if (process.env.PI_EXT_DEBUG_INVOKE) process.stderr.write(`[dbg] custom handled t=${Date.now()}\n`);
      const rendered = activeCustom.component.render(Number(params?.width ?? 80));
      respond(id, {
        lines: rendered.map((line) => String(line)),
        closed: activeCustom.isClosed(),
        ...(activeCustom.isClosed() ? { result: activeCustom.result() ?? null } : {}),
      });
    } catch (error) {
      respond(id, {
        lines: [],
        error: String(error && error.message ? error.message : error),
      });
    }
    return;
  }
  if (method === "ext.markdown.transform") {
    // ref `interactive-mode.ts:2137-2139` 的 `getMarkdownTransformers()`：
    // 按注册顺序**链式**应用（前一个的输出是后一个的输入）。
    // 每个 transformer 的签名是 `(markdown, context) => markdown`
    // （`types.ts:1304`），`context` 目前给 `{availableWidth}`。
    let text = String(params?.markdown ?? "");
    const context = { availableWidth: Number(params?.width ?? 80) };
    const errors = [];
    for (const transformer of markdownTransformers) {
      try {
        const next = transformer(text, context);
        if (typeof next === "string") text = next;
      } catch (error) {
        errors.push(String(error && error.message ? error.message : error));
      }
    }
    respond(id, { markdown: text, ...(errors.length > 0 ? { errors } : {}) });
    return;
  }
  if (method === "ext.render.thinking.batch") {
    const labels = (params?.items ?? []).map((item) => {
      try {
        const component = new shimCodingAgent.AssistantMessageComponent();
        component.hideThinkingBlock = item?.hidden !== false;
        const content = (item?.content ?? []).filter((block) => block?.type === "thinking");
        const message = { role: "assistant", content };
        if (typeof component.updateContent === "function") {
          component.updateContent(message, false);
        }
        return typeof component.hiddenThinkingLabel === "string"
          ? component.hiddenThinkingLabel
          : null;
      } catch {
        return null;
      }
    });
    respond(id, { labels });
    return;
  }
  if (method === "ext.render.thinking") {
    // F51/Z1：把**思考块的折叠标签**交给扩展决定（ref 的
    // `AssistantMessageComponent.updateContent` 原型补丁，见
    // `pi-ai-summary/src/adapters/thinking-label.ts:489-526`）。
    // 我们建一个组件实例、让它跑扩展的 `updateContent`，再读回
    // `hiddenThinkingLabel`；扩展没装/没给标签 ⇒ `label:null`（Rust 用原文）。
    try {
      const component = new shimCodingAgent.AssistantMessageComponent();
      // F52/B：请求里带 `hidden`（只剩隐藏态会用到这个标签），默认 true 与被测语义一致。
      component.hideThinkingBlock = params?.hidden !== false;
      const content = (params?.content ?? []).filter((block) => block?.type === "thinking");
        const message = { role: "assistant", content };
      if (typeof component.updateContent === "function") {
        component.updateContent(message, false);
      }
      const label =
        typeof component.hiddenThinkingLabel === "string" ? component.hiddenThinkingLabel : null;
      respond(id, { label });
    } catch (error) {
      respond(id, { label: null, error: String(error?.message ?? error) });
    }
    return;
  }
  if (method === "ext.tool.known") {
    // F51/Z0：该工具是否有定义（决定"只有名字"还是"名字 + 缩进 JSON 参数"）。
    const name = String(params?.toolName ?? "");
    respond(id, { known: tools.has(name) });
    return;
  }
  if (method === "ext.render") {
    // 渲染钩子降级（ref `tool-execution.ts:26-33`）：跑扩展的 renderCall/renderResult，
    // 把返回的组件序列化成帧；不支持的组件/抛错 ⇒ `unsupported`（宿主回落内置渲染）。
    const toolName = params?.toolName;
    const definition = tools.get(toolName);
    const phase = params?.phase === "result" ? "result" : "call";
    const renderer = definition?.[phase === "call" ? "renderCall" : "renderResult"];
    if (typeof renderer !== "function") {
      // F51/Z0：把"**这个工具是否有定义**"一并回给宿主。
      //
      // ref `components/tool-execution.ts:316-320`：有定义但**没有** `renderCall` 时走
      // `createCallFallback()` = 只有工具名；**完全没有定义**（`hasRendererDefinition()`
      // 为 false）时走 `formatToolExecution()`，即"名字 + 缩进 JSON 参数"。两者在屏幕上
      // 完全不同（` report_to_user` + 结果文本 vs ` zeta_broken` + JSON 块 + 结果文本），
      // 所以宿主必须能区分。
      respond(id, { frames: null });
      return;
    }
    const themeStub = makeThemeStub();
    const ctxStub = {
      args: params?.args ?? {},
      toolCallId: "ext",
      cwd: hostCtx.cwd,
      isPartial: false,
      expanded: Boolean(params?.options?.expanded),
      invalidate() {},
      state: {},
    };
    try {
      const component =
        phase === "call"
          ? renderer(params?.args ?? {}, themeStub, ctxStub)
          : renderer(params?.result ?? {}, params?.options ?? {}, themeStub, ctxStub);
      respond(id, { frames: shimTui.frameOf(component) });
    } catch (error) {
      respond(id, {
        frames: {
          kind: "unsupported",
          reason: String(error && error.message ? error.message : error),
        },
      });
    }
    return;
  }
  if (method === "ext.render.tool") {
    // F48/X3 + F50/Z1：把工具行渲染委托给扩展的**原型补丁**（ref 机制见
    // `pi-ai-summary/src/adapters/tool-render.ts:246-252`：扩展直接替换
    // `ToolExecutionComponent.prototype.render`）。
    //
    // **批量**（F50/Z1）：`params.rows` 是数组时一次处理全部行，只回一条响应 ⇒ 宿主侧
    // 只重绘一次。逐条请求会让"视口上方的行"每变一次就触发一次整屏重绘
    // （ref `tui-main-screen.ts:455-459`），历史工具行越多重绘次数越多（真机"反复刷新"）。
    const proto = shimCodingAgent.ToolExecutionComponent?.prototype;
    if (typeof proto?.render !== "function") {
      const rows = Array.isArray(params?.rows) ? params.rows : [params];
      respond(id, {
        rows: rows.map((row) => ({ id: String(row?.toolCallId ?? ""), lines: null })),
        error: "ToolExecutionComponent.prototype.render 不可用",
      });
      return;
    }
    const widthOf = (row) => Number(row?.width ?? params?.width ?? 80) || 80;
    const rows = Array.isArray(params?.rows) ? params.rows : [params];
    const out = [];
    for (const row of rows) {
      const width = widthOf(row);
      const key = String(row?.toolCallId ?? row?.toolName ?? "");
      try {
        const component = new shimCodingAgent.ToolExecutionComponent(
          String(row?.toolName ?? ""),
          key,
          row?.args ?? undefined,
          {},
          undefined,
          undefined,
          hostCtx.cwd,
        );
        component.isPartial = row?.isPartial !== false;
        component.expanded = row?.expanded === true;
        component.result = row?.result ?? undefined;
        const startedAt = Number(row?.startedAt);
        const endedAt = Number(row?.endedAt);
        component.rendererState = {
          ...(Number.isFinite(startedAt) ? { startedAt } : {}),
          ...(Number.isFinite(endedAt) ? { endedAt } : {}),
        };
        // ref `:307`：渲染前先 `updateDisplay()`（扩展常把它当作缓存失效点）。
        if (typeof component.updateDisplay === "function") component.updateDisplay();
        let rendered;
        try {
          rendered = component.render(width);
        } finally {
          const logPath = process.env.PI_TOOL_ROW_LOG;
          if (logPath) {
            try {
              const fsMod = await import("node:fs");
              fsMod.appendFileSync(
                logPath,
                `${key} rendered=${rendered === null ? "null" : Array.isArray(rendered) ? `arr(${rendered.length})` : typeof rendered} original=${component.__originalRenderCalled === true}\n`,
              );
            } catch {
              // 诊断写盘失败不影响渲染。
            }
          }
        }
        if (rendered === null || rendered === undefined) {
          // 没人接管（默认实现）⇒ Rust 原生渲染。
          // `fallback:true` = 扩展接管过、但又调回了 originalRender（内部 catch 吞了异常），
          // 与"根本没有补丁"要分开，否则归因时会误判（F51 实测 463 行只折叠 2 行）。
          out.push({ id: key, lines: null, fallback: component.__originalRenderCalled === true });
        } else if (!Array.isArray(rendered)) {
          out.push({ id: key, lines: null, error: "render() 未返回 string[]" });
        } else {
          out.push({ id: key, lines: rendered.map((line) => String(line)) });
        }
      } catch (error) {
        out.push({ id: key, lines: null, error: String(error?.message ?? error) });
      }
    }
    respond(id, { rows: out });
    return;
  }
  if (method === "ext.render.footer" || method === "ext.render.widget") {
    // F38/K4：按宿主给的 `width` 与 `digest` 重跑组件 `render(width)`。
    // 结果按 `(kind, key, width, digest)` 缓存（同一状态不重复跑）；超时/抛错由宿主兜底。
    const kind = method === "ext.render.footer" ? "footer" : "widget";
    const key = kind === "footer" ? "" : String(params?.key ?? "");
    const width = Number(params?.width ?? 80) || 80;
    const digest = String(params?.digest ?? "");
    const cached = renderCache.get(cacheKey(kind, key, width, digest));
    if (cached) {
      respond(id, { lines: cached, cached: true });
      return;
    }
    const factory = kind === "footer" ? footerFactory : widgetFactories.get(key);
    if (typeof factory !== "function") {
      respond(id, { lines: null });
      return;
    }
    try {
      const rendered =
        kind === "footer"
          ? factory({ requestRender() {} }, makeThemeStub(), footerDataStub())?.render?.(width)
          : factory({ requestRender() {} }, makeThemeStub())?.render?.(width);
      if (!Array.isArray(rendered)) {
        respond(id, { lines: null, error: "render() did not return string[]" });
        return;
      }
      const lines = rendered.map((line) => String(line));
      rememberRender(cacheKey(kind, key, width, digest), lines);
      respond(id, { lines });
    } catch (error) {
      respond(id, { lines: null, error: String(error?.message ?? error) });
    }
    return;
  }
  if (method === "ext.command.invoke") {
    const name = params?.commandName;
    // 宿主传的是 invocationName（重名时带 `:N`）；回落到底层注册名。
    const baseName = typeof name === "string" ? name.replace(/:\d+$/, "") : name;
    const handler = commandHandlers.get(name) ?? commandHandlers.get(baseName);
    if (!handler) {
      respond(id, undefined, { code: -32601, message: `unknown command ${name}` });
      return;
    }
    try {
      await handler(String(params?.args ?? ""), contextForExtension());
      respond(id, { ok: true });
    } catch (error) {
      const message = String(error && error.message ? error.message : error);
      process.stderr.write(`[ext-host] command ${name} failed: ${message}\n`);
      respond(id, { ok: false, error: message });
    }
    return;
  }
  if (method === "ext.event") {
    const name = params?.name;
    // 回合/会话状态变化时丢弃 UI 渲染缓存（ref 是**每帧**调组件 `render(width)`，
    // 所以扩展在 `turn_end` 里自增的私有计数下一帧就可见）。
    //
    // 宿主的缓存键是 `(kind, key, width, digest)`，而 digest 只看**宿主可见**状态，
    // 看不到扩展自己的变量 ⇒ 不在这里清的话，`render(width)` 根本不会被再调用，
    // 画面永远停在旧值（`T97-footer-refresh-on-state` 实测：footer 被请求渲染 6 次、
    // 每次都返回缓存的 `K4 n=0`，而扩展内部的 `turns` 已经数到 4）。
    if (
      name === "turn_end" ||
      name === "agent_end" ||
      name === "agent_settled" ||
      name === "session_start" ||
      name === "session_info_changed" ||
      name === "thinking_level_select" ||
      name === "model_select"
    ) {
      renderCache.clear();
    }
    applyHostCtx(params?.ctx);
    const list = [...(handlers.get(name) ?? [])];
    const errors = [];
    // 有返回值语义的两类事件（ref `runner.ts:1136-1153` 的 emitToolCall、
    // `runner.ts:1080-1135` 的 emitToolResult）：
    // - tool_call：按顺序跑，**第一个 block 立刻返回**（后续处理器不再执行）；
    // - tool_result：链式——每个处理器看到前一个改过的 event；某个处理器抛错只记录。
    // `context`：请求前的消息变换（ref `runner.ts:1190-1219` 的 `emitContext`）。
    // 扩展可以返回 `{messages}` 新列表，也可以**原地改** `event.messages`；
    // system 消息由宿主还原（这里只处理传进来的非 system 列表）。
    if (name === "context") {
      let current = Array.isArray(params?.payload?.messages) ? params.payload.messages.slice() : [];
      const snapshot = current.slice();
      for (const handler of list) {
        try {
          const event = { type: "context", messages: current };
          const result = await handler(event, contextForExtension());
          if (result && Array.isArray(result.messages)) {
            current = result.messages.slice();
          } else if (event.messages !== current) {
            // 处理器把 `event.messages` 换成了新数组。
            current = event.messages;
          } else if (current.some((message, index) => message !== snapshot[index])) {
            // 原地改了元素（对象引用变化或字段变化）——保持当前数组。
            current = current.slice();
          }
        } catch (error) {
          errors.push(String(error && error.message ? error.message : error));
          process.stderr.write(`[ext-host] context handler failed: ${errors[errors.length - 1]}\n`);
        }
      }
      respond(id, { handled: list.length, errors, result: { messages: current } });
      return;
    }
    if (name === "tool_call" || name === "tool_result") {
      let current = { ...(params?.payload ?? {}) };
      // **就地修改**的载体（ref `agent-loop.ts:727` + `agent-session.ts:538-543`）：
      // 扩展拿到的是同一个 `input` 对象，改它等于改宿主要执行的参数；把**这个对象**
      // （而不是 `current.input`，那可能是被整体替换过的新对象）回传，宿主据此替换参数。
      // 只对 `tool_call` 生效（`tool_result` 没有参数语义）。
      const originalInput = current.input;
      let merged;
      for (const handler of list) {
        try {
          const result = await handler(current, contextForExtension());
          if (name === "tool_call") {
            if (result) {
              merged = result;
              if (result.block) break;
            }
          } else if (name === "tool_result") {
            // **没返回值 = 只观察，不改写**（ref `runner.ts:1091` 的
            // `if (!handlerResult) continue;`）。少这一句时，任何"只看看"的
            // `tool_result` 处理器都会让这里读到 `undefined.content` 而抛
            // `Cannot read properties of undefined (reading 'content')`，
            // 于是该扩展的 tool_result 钩子整体失效（真机反馈，Windows 版同样）。
            if (!result) continue;
            if (result.content !== undefined) current.content = result.content;
            if (result.details !== undefined) current.details = result.details;
            if (result.isError !== undefined) current.isError = result.isError;
            if (result.usage !== undefined) current.usage = result.usage;
            merged = {
              content: current.content,
              details: current.details,
              isError: current.isError,
              ...(current.usage !== undefined ? { usage: current.usage } : {}),
            };
          } else if (result) {
            // **有返回值的其它事件**（ref `extensionRunner.emit` 直接把 handler 的
            // 返回值交给调用方）：目前是 `session_before_switch`（`{cancelled}`）与
            // `session_before_tree`（`{cancel, summary}`）。
            //
            // 此前这里只处理 `tool_call`/`tool_result`，其余事件的返回值被**丢弃**
            // ⇒ 宿主永远收到 `result: null`，扩展的"取消导航/提供摘要"完全无效
            // （实测：handler 返回 `{cancel:true}`，宿主读到 `cancel:false`）。
            merged = result;
          }
        } catch (error) {
          const message = String(error && error.message ? error.message : error);
          errors.push(message);
          process.stderr.write(`[ext-host] ${name} handler failed: ${message}\n`);
        }
      }
      // 与 ref 对齐：返回 `{input}` **不生效**，只有"就地改过的原对象"生效
      // （`agent-loop.ts:727`/`:770-775`：宿主执行的是同一个 `args`；`runner.ts:1134-1152`
      // 只用返回值的 `block/reason/terminate`）。所以把原对象放进 `result.input` 覆盖掉
      // 处理器返回的同名键——这是"忽略替换、接受就地修改"的最短表达。
      const result =
        name === "tool_call" && originalInput !== undefined
          ? { ...(merged ?? {}), input: originalInput }
          : (merged ?? null);
      respond(id, { handled: list.length, errors, result });
      return;
    }
    // `input`：链式 transform，`handled` 立即短路（ref `runner.ts:1412-1460`）。
    if (name === "input") {
      let current = { ...(params?.payload ?? {}) };
      const original = { text: current.text, images: current.images };
      for (const handler of list) {
        try {
          const result = await handler(current, contextForExtension());
          if (result?.action === "handled") {
            respond(id, { handled: list.length, errors, result: { action: "handled" } });
            return;
          }
          if (result?.action === "transform") {
            if (typeof result.text === "string") current.text = result.text;
            if (result.images !== undefined) current.images = result.images;
            current = { ...current, text: current.text, images: current.images };
          }
        } catch (error) {
          errors.push(String(error && error.message ? error.message : error));
        }
      }
      const changed = current.text !== original.text || current.images !== original.images;
      respond(id, {
        handled: list.length,
        errors,
        result: changed
          ? { action: "transform", text: current.text, images: current.images }
          : { action: "continue" },
      });
      return;
    }
    // `user_bash`：**第一个**返回 `{result}` 或 `{operations}` 的处理器胜出；
    // 返回 `undefined` 继续下一个；**任一处理器抛错即整体失败**（ref `runner.ts:1154-1180`）。
    if (name === "user_bash") {
      for (const handler of list) {
        let result;
        try {
          result = await handler({ ...(params?.payload ?? {}) }, contextForExtension());
        } catch (error) {
          const message = String(error && error.message ? error.message : error);
          process.stderr.write(`[ext-host] user_bash handler failed: ${message}\n`);
          respond(id, { handled: list.length, errors: [message], result: { __failed: message } });
          return;
        }
        if (result === undefined) continue;
        const valid =
          result &&
          typeof result === "object" &&
          ((result.result && typeof result.result === "object") ||
            (result.operations && typeof result.operations === "object"));
        if (!valid) {
          respond(id, {
            handled: list.length,
            errors,
            result: { __failed: "Invalid user_bash handler result" },
          });
          return;
        }
        respond(id, { handled: list.length, errors, result });
        return;
      }
      respond(id, { handled: list.length, errors, result: null });
      return;
    }
    // `project_trust`：**第一个**给出 `yes`/`no` 的处理器胜出并立即返回；
    // `undecided`（或缺省）继续问下一个（ref `runner.ts:292-322` 的
    // `emitProjectTrustEvent`）。与 `context` 的链式叠加刻意不同——
    // 信任决策要确定性，不能把多个扩展的意见拼在一起。
    if (name === "project_trust") {
      for (const handler of list) {
        try {
          const result = await handler(params?.payload ?? {}, contextForExtension());
          const trusted = result && result.trusted;
          if (trusted === "yes" || trusted === "no") {
            respond(id, {
              handled: list.length,
              errors,
              result: { trusted, remember: result.remember === true },
            });
            return;
          }
        } catch (error) {
          errors.push(String(error && error.message ? error.message : error));
        }
      }
      respond(id, { handled: list.length, errors, result: null });
      return;
    }
    // `before_agent_start`：收集 `message`，`systemPrompt` 覆盖后者胜（ref `runner.ts:1312-1365`）。
    if (name === "before_agent_start") {
      const messages = [];
      let systemPrompt;
      // **`systemPromptOptions` 必须在多个 handler 之间共享同一份可变对象**：
      // ref `types.ts:745-746` 明确"Later handlers observe mutations made by earlier
      // handlers"。此前这里只收集 `message`/`systemPrompt`，把
      // `event.systemPromptOptions.sections` 的改动整个丢掉了 ⇒
      // `pi-handoff-gate`（`index.ts:557` 往 sections 里写自己的键）追加的
      // 系统提示段完全不生效。
      const options = JSON.parse(JSON.stringify(params?.payload?.systemPromptOptions ?? {}));
      for (const handler of list) {
        try {
          const result = await handler(
            { ...(params?.payload ?? {}), systemPromptOptions: options },
            contextForExtension(),
          );
          if (result?.message) messages.push(result.message);
          if (result?.systemPrompt !== undefined) systemPrompt = result.systemPrompt;
          // 处理器也可能整体**返回**新的 options（本实现兼容两种写法）。
          if (result?.systemPromptOptions && typeof result.systemPromptOptions === "object") {
            Object.assign(options, result.systemPromptOptions);
          }
        } catch (error) {
          errors.push(String(error && error.message ? error.message : error));
        }
      }
      respond(id, {
        handled: list.length,
        errors,
        result: {
          messages,
          ...(systemPrompt !== undefined ? { systemPrompt } : {}),
          systemPromptOptions: options,
        },
      });
      return;
    }
    // `agent_before_settle`：entries/continue 链式累积（ref `runner.ts:928-975`）。
    if (name === "agent_before_settle") {
      let entries = [];
      let shouldContinue = false;
      for (const handler of list) {
        try {
          const result = await handler(
            { ...(params?.payload ?? {}), entries, continue: shouldContinue },
            contextForExtension(),
          );
          if (result?.entries !== undefined) entries = result.entries;
          if (result?.continue !== undefined) shouldContinue = result.continue;
        } catch (error) {
          errors.push(String(error && error.message ? error.message : error));
        }
      }
      respond(id, { handled: list.length, errors, result: { entries, continue: shouldContinue } });
      return;
    }
    // **有返回值语义的其余事件**：`session_before_switch`（`{cancelled}`）与
    // `session_before_tree`（`{cancel, summary}`）。ref 的 `extensionRunner.emit`
    // 直接把 handler 的返回值交给调用方。
    //
    // 此前这里 `await handler(...)` **丢弃返回值** ⇒ 宿主永远收到 `result: null`，
    // 扩展的"取消导航/给摘要/取消切换"全部无效（实测：handler 返回 `{cancel:true}`，
    // 宿主读到 `cancel:false`）。多个 handler 时**后者的非空字段覆盖前者**，
    // 与 ref 遍历所有 handler 的语义一致。
    let merged;
    for (const handler of list) {
      try {
        const result = await handler(params?.payload ?? {}, contextForExtension());
        if (result && typeof result === "object") {
          merged = { ...(merged ?? {}), ...result };
        }
      } catch (error) {
        errors.push(String(error && error.message ? error.message : error));
      }
    }
    if (errors.length > 0) {
      process.stderr.write(`[ext-host] event ${name}: ${errors.length} handler(s) failed: ${errors.join("; ")}\n`);
    }
    respond(id, { handled: list.length, errors, result: merged ?? null });
    return;
  }
  if (method === "ext.flags.values") {
    const values = params?.values;
    flagValues = values && typeof values === "object" ? values : {};
    respond(id, { ok: true });
    return;
  }
  if (method === "host.shutdown") {
    respond(id, { ok: true });
    process.exit(0);
  }
  respond(id, undefined, { code: -32601, message: `method not implemented: ${method}` });
}

function normalizeResult(value) {
  if (value && Array.isArray(value.content)) return value;
  return { content: [{ type: "text", text: JSON.stringify(value) }] };
}

// ---- 组装扩展可见的 `pi` API ----
// 注册类：**记录**（本批只用到 registerTool/registerCommand/registerFlag/on）；
// 其余 API：给"安全的空实现"，绝不返回 undefined 让扩展崩在 `pi.xxx is not a function`
// （P9-1-B 的教训：宁可有记录的 no-op，也不要抛在工厂期把整个扩展拖死）。
const warnings = [];
function noteNotImplemented(name) {
  warnings.push(name);
  process.stderr.write(`[ext-host] ${name} 未实现（本批），已按 no-op 处理\n`);
}

/// 扩展注册的 Markdown 变换器（按加载顺序；ref `runner.ts:725-727`）。
const markdownTransformers = [];

/// 扩展间共享事件总线（ref `event-bus.ts:12-31` 的 `createEventBus`）。
///
/// 用 Node 的 `EventEmitter`，并按 ref 的 `safeHandler` 包一层：
/// 处理器抛错只记 `console.error`，**不打断其它订阅者**（ref 同款）。
const eventBus = (() => {
  const emitter = new EventEmitter();
  return {
    emit: (channel, data) => {
      emitter.emit(channel, data);
    },
    on: (channel, handler) => {
      const safeHandler = async (data) => {
        try {
          await handler(data);
        } catch (err) {
          console.error(`Event handler error (${channel}):`, err);
        }
      };
      emitter.on(channel, safeHandler);
      return () => emitter.off(channel, safeHandler);
    },
  };
})();

const pi = {
  on(event, handler) {
    if (!events.includes(event)) events.push(event);
    if (typeof handler !== "function") return () => {};
    const list = handlers.get(event) ?? [];
    list.push(handler);
    handlers.set(event, list);
    // unsubscribe：摘掉这个处理器（ref `ExtensionAPI.on` 返回 `() => void`）。
    return () => {
      const current = handlers.get(event) ?? [];
      const index = current.indexOf(handler);
      if (index >= 0) current.splice(index, 1);
      handlers.set(event, current);
    };
  },
  registerTool(tool) {
    if (!tool || typeof tool.name !== "string" || !tool.name) {
      throw new Error("registerTool: tool.name is required");
    }
    tools.set(tool.name, tool);
  },
  registerCommand(name, definition) {
    if (typeof name !== "string") return;
    const description = typeof definition?.description === "string" ? definition.description : undefined;
    if (!commands.some((command) => command.name === name)) {
      commands.push({
        name,
        description,
        scope: scopeTagForPath(currentExtensionPath || process.cwd()),
        sourceInfo: {
          path: path.resolve(currentExtensionPath || process.cwd()),
          source: "local",
          scope: "temporary",
          origin: "top-level",
          baseDir: path.dirname(path.resolve(currentExtensionPath || process.cwd())),
        },
      });
    }
    if (definition && typeof definition.handler === "function") {
      commandHandlers.set(name, definition.handler);
    }
  },
  // ref `types.ts:1499`：把一条自定义条目写进会话（`{type:"custom", customType, data}`）。
  appendEntry(customType, data) {
    return send("host.session.appendEntry", {
      customType: String(customType ?? "custom"),
      data: data === undefined ? null : data,
    });
  },
  registerFlag(name, definition) {
    if (typeof name !== "string") return;
    const type = definition?.type === "string" ? "string" : "boolean";
    flagSpecs.set(name, { type, default: definition?.default });
  },
  // ref：`getFlag(name)` 取 CLI 解析后的值，缺省回落到注册默认值。
  getFlag(name) {
    if (Object.prototype.hasOwnProperty.call(flagValues, name)) return flagValues[name];
    return flagSpecs.get(name)?.default;
  },
  registerShortcut() {
    noteNotImplemented("pi.registerShortcut");
  },
  registerMessageRenderer() {
    noteNotImplemented("pi.registerMessageRenderer");
  },
  registerEntryRenderer() {
    noteNotImplemented("pi.registerEntryRenderer");
  },
  // ref `types.ts:1473` 的 `registerMarkdownTransformer`：把用户/助手 Markdown
  // **渲染前**交给扩展改写。
  //
  // 真实使用方是 `pi-secrets`（用户已装）：`pi.registerMarkdownTransformer((md) =>
  // redactText("markdown", md))`（`src/index.ts:326`）—— 它把命中的密钥替换成掩码，
  // 靠这个钩子保证**密钥不出现在终端 scrollback 里**（注释写明"message_end/tool_result
  // 之前就已经流式渲染，必须有自己的钩子"）。
  //
  // 此前这里只 `void transformer`（记录不应用）⇒ 脱敏**完全失效**：
  // 密钥会原样画到屏幕上。这是比"功能缺失"更严重的问题（属于信息泄露），
  // 因此本批实现它。
  registerMarkdownTransformer(transformer) {
    if (typeof transformer === "function") markdownTransformers.push(transformer);
  },
  registerProvider() {
    noteNotImplemented("pi.registerProvider");
  },
  unregisterProvider() {
    // 与 `registerProvider` 对称：本实现没有运行期 provider 注册表，
    // 因此这里**不能**做成空实现——扩展会以为注销成功了（静默失败）。
    noteNotImplemented("pi.unregisterProvider");
  },
  // ref `types.ts:1506` 的 `setSessionName(name)`：写 `session_info` 条目并通告。
  // 同步更新本地缓存，让紧随其后的 `getSessionName()` 读到新值（ref 是同步语义）。
  setSessionName(name) {
    const value = String(name ?? "");
    hostCtx = { ...hostCtx, sessionName: value === "" ? undefined : value };
    // 失败不抛：ref 的 `setSessionName` 也没有返回/抛错契约；错误已在宿主侧记录。
    void send("host.session.setName", { name: value }).catch(() => {});
  },
  // ref `types.ts:1508-1509`：当前会话名。值来自宿主下发的 ctx 快照
  // （`host.session.setName` 成功后宿主会 `push_ctx` 再下发）。
  getSessionName() {
    return hostCtx.sessionName ?? undefined;
  },
  sendMessage() {
    noteNotImplemented("pi.sendMessage");
  },
  // ref `types.ts:1517` 一类：把一条用户消息排进队列（宿主在下一轮注入）。
  sendUserMessage(text) {
    return send("host.session.sendUserMessage", { text: String(text ?? "") });
  },
  async exec() {
    noteNotImplemented("pi.exec");
    return { stdout: "", stderr: "", exitCode: 1 };
  },
  getActiveTools() {
    return [];
  },
  // ref `agent-session.ts:1259-1267`：返回**全部**工具定义（内置 + 扩展）。
  // 此前只返回扩展自己注册的 → `pi-secrets` 统计内置 `bash` 时恒为 0。
  // 内置部分由宿主经 `ctx.builtinTools` 下发（`set_ctx_builtin_tools`），
  // 扩展注册的**同名覆盖**内置（ref 的工具注册表以名字为键）。
  getAllTools() {
    const extensionTools = [...tools.values()].map((tool) => ({
      name: tool.name,
      description: tool.description ?? "",
      parameters: tool.parameters ?? { type: "object", properties: {} },
      promptGuidelines: tool.promptGuidelines ?? [],
      sourceInfo: { kind: "extension" },
    }));
    const extensionNames = new Set(extensionTools.map((tool) => tool.name));
    const builtins = Array.isArray(hostCtx.builtinTools)
      ? hostCtx.builtinTools.filter((tool) => !extensionNames.has(tool?.name))
      : [];
    return [...builtins, ...extensionTools];
  },
  setActiveTools() {},
  // ref `agent-session.ts:3021-3044`：**扩展命令 + prompt 模板 + 技能**三类。
  // 技能名带 `skill:` 前缀（`name: \`skill:${skill.name}\``）。模板与技能清单由宿主
  // 下发（`set_ctx_prompt_commands` / `set_ctx_skill_commands`）。
  getCommands() {
    const extensionCommands = commands.map((command) => ({
      name: command.name,
      description: command.description,
      source: "extension",
    }));
    const templates = Array.isArray(hostCtx.promptCommands)
      ? hostCtx.promptCommands.map((item) => ({
          name: item?.name,
          description: item?.description ?? "",
          source: "prompt",
        }))
      : [];
    const skills = Array.isArray(hostCtx.skillCommands)
      ? hostCtx.skillCommands.map((item) => ({
          name: `skill:${item?.name}`,
          description: item?.description ?? "",
          source: "skill",
        }))
      : [];
    return [...extensionCommands, ...templates, ...skills];
  },
  async setModel() {
    noteNotImplemented("pi.setModel");
    return false;
  },
  // ref `types.ts:1538-1546`：读当前档位；写会按模型能力夹取后写会话条目。
  // 值与 `pi.getSessionName` 同源——宿主下发的 ctx 快照（`push_ctx`）。
  // ref `types.ts:352`：当前生效的系统提示。值来自宿主下发的 ctx 快照。
  getSystemPrompt() {
    return hostCtx.systemPrompt ?? "";
  },
  getThinkingLevel() {
    return hostCtx.thinkingLevel ?? "off";
  },
  setThinkingLevel(level) {
    const value = String(level ?? "off");
    hostCtx = { ...hostCtx, thinkingLevel: value };
    void send("host.thinking.setLevel", { level: value }).catch(() => {});
  },
  // ref `agent-session.ts:1279-1291` 的 `setActiveToolsByName`：宿主按给的名字
  // 重建工具集（不在注册表里的名字被忽略），下一次请求体与 `toolsAdded` 立刻反映。
  // ref `agent-session.ts:3079-3081`：写一条 `label` 条目给指定 entry 打标记。
  // `label` 为 undefined/null/空串 ⇒ 清除标记（ref 写 `label: undefined`）。
  setLabel(entryId, label) {
    const id = String(entryId ?? "");
    const value = label === undefined || label === null ? null : String(label);
    void send("host.session.setLabel", { entryId: id, label: value }).catch(() => {});
  },
  setActiveTools(toolNames) {
    const names = Array.isArray(toolNames) ? toolNames.map((n) => String(n)) : [];
    void send("host.tools.setActive", { names }).catch(() => {});
  },
  // ref `event-bus.ts:3-31` 的 `EventBus`：扩展间**共享事件总线**。
  //
  // 接口是 `emit(channel, data)` / `on(channel, handler) -> unsubscribe`
  // （此前这里只有空实现 `publish(){}` + `on(){return ()=>{}}`，
  // 且方法名都不对 —— 扩展调用 `pi.events.emit(...)` 会直接 `TypeError`）。
  //
  // 真实使用方（已装扩展）：
  // - `pi-handoff-gate`：`pi.events.on(RESILIENCE_PRESENT_CHANNEL, …)`、
  //   `on(RESILIENCE_AUTO_RESUME_CHANNEL, …)`（`index.ts:235-240`）——与
  //   `pi-agent-resilience` 协作，避免双方同时处理同一空响应边界；
  // - `pi-agent-resilience`：`pi.events.emit(...)` 广播"我在"与"我要自动恢复"；
  // - `pi-interact` ↔ `pi-remote-control`：用 `pi.remote-control.ui.probe.v1`
  //   做同步握手（`ui-bridge.ts:4-6`、`adapter.ts:189`）。
  // 总线是**空实现**时这些协作全部静默失效（扩展之间互相看不见）。
  events: {
    emit(channel, data) {
      eventBus.emit(String(channel), data);
    },
    on(channel, handler) {
      return eventBus.on(String(channel), handler);
    },
  },
  ui: {
    notify(message) {
      process.stderr.write(`[ext] ${message}\n`);
    },
    setStatus() {},
    setWidget() {
      throw new Error("pi-ext-host: ctx.ui.setWidget 未实现（P9 后续批次）");
    },
    async select() {
      throw new Error("pi-ext-host: ctx.ui.select 未实现（P9 后续批次）");
    },
    async input() {
      throw new Error("pi-ext-host: ctx.ui.input 未实现（P9 后续批次）");
    },
  },
};

// jiti 解析放在独立模块（纯函数，可单测；见 `jiti.mjs`）。
import {
  extensionImportUrl,
  extensionLabel,
  resolveJiti,
} from "./jiti.mjs";

// 与 ref `core/extensions/virtual-modules.ts:12-40` 同名映射。
const VIRTUAL_MODULES = {
  typebox: shimTypebox,
  "typebox/compile": shimTypebox,
  "typebox/value": shimTypebox,
  "@sinclair/typebox": shimTypebox,
  "@earendil-works/pi-tui": shimTui,
  "@earendil-works/pi-coding-agent": shimCodingAgent,
  "@earendil-works/pi-ai": shimAi,
  "@mariozechner/pi-tui": shimTui,
  "@mariozechner/pi-coding-agent": shimCodingAgent,
  "@mariozechner/pi-ai": shimAi,
};

const searchAttempts = [];

async function makeJiti(extensionPaths) {
  // F34/W1d：把搜索过的候选目录留档，失败提示里一次列全（真机一次定位）。
  const found = resolveJiti(extensionPaths, { attempts: searchAttempts });
  if (!found) return undefined;
  const require = createRequire(import.meta.url);
  const module = require(found.entry);
  const createJiti = module.createJiti ?? module.default?.createJiti ?? module.default ?? module;
  if (typeof createJiti !== "function") return undefined;
  // 选项逐条照 ref `core/extensions/loader.ts:496-501`：
  // `moduleCache: false`、`virtualModules`、`tryNative: false`（embedded 分支）。
  return createJiti(import.meta.url, {
    moduleCache: false,
    virtualModules: VIRTUAL_MODULES,
    tryNative: false,
  });
}

// 给 shim 用的"宿主动作"回调（`createBashToolDefinition` 等的执行由宿主完成）。
// 最小 theme：`fg` 用 ref `theme/dark.json` 的色表（`theme.ts:323-327`：**未知色名直接抛错**，
// 不静默回落）。`bg/bold/dim/italic` 目前直通文本（未实现背景色，见 LIMITATIONS）。
// `ReadonlyFooterDataProvider` 的最小面（ref `core/footer-data-provider.ts:103-156`）：
// 只给扩展实际用到的读取方法，缺的显式抛错。
let activeCustom = null;

// `KeybindingsManager` 的最小面：`matches()` 走与 `matchesKey` 同一张表（未知名抛错，不静默 false）。
function keybindingsStub() {
  return {
    matches: (data, binding) => shimTui.matchesKey(data, binding),
  };
}

function footerDataStub() {
  return {
    // 与 `ctx.ui.setStatus` 共用同一张表（ref `footer.ts` 的 `extensionStatuses`）。
    getExtensionStatuses: () => new Map(extensionStatuses),
    // ref `footer.ts` 的 `footerData.getAvailableProviderCount()`：>1 时右侧加 `(provider)` 前缀。
    getAvailableProviderCount: () =>
      typeof hostCtx.availableProviderCount === "number" ? hostCtx.availableProviderCount : 1,
    // ref `core/footer-data-provider.ts:239-248`：宿主提供当前 git 分支（无仓库 ⇒ undefined）。
    // 此前恒为 `undefined` ⇒ 扩展页脚（pi-ai-summary）**永远丢分支名**（真机 G1，实测）。
    getGitBranch: () =>
      typeof hostCtx.gitBranch === "string" && hostCtx.gitBranch.length > 0
        ? hostCtx.gitBranch
        : undefined,
    getCwd: () => hostCtx.cwd,
    onBranchChange: () => () => {},
  };
}

// `makeThemeStub` 的实现已移到 `shim/theme-colors.mjs`（纯函数，便于 `node --test`
// 直接覆盖样式行为）。**必须保持 `function` 声明**：本文件里它在第 346 行就被调用，
// 而定义在 1434 行——`const` 箭头函数会落进 TDZ（暂存死区）并抛 `ReferenceError`，
// 表现为 `ctx.ui.theme` 全不可用、footer/widget 渲染静默失败。
function makeThemeStub() {
  return shimTheme.makeThemeStub(hostCtx);
}

globalThis.__piHostCall = (action, args) =>
  send("host.action.call", { action: String(action), args: args ?? {} });

async function loadExtension(extensionPath, jiti) {
  if (/\.(ts|mts)$/i.test(extensionPath)) {
    if (!jiti) {
      throw new Error(
        "需要 jiti 才能加载 .ts 扩展；未找到可用 jiti（可用 PI_JITI 指向 jiti 包目录）",
      );
    }
    return jiti.import(extensionPath, { default: true });
  }
  // F35/G4a：Windows 绝对路径必须转 `file://` URL（裸 `C:\a\b.mjs` 会被当成协议）。
  return import(extensionImportUrl(extensionPath));
}

async function main() {
  await send("ext.hello", {
    token,
    protocolVersion,
    node: process.version,
    piVersion: "0.87.1",
    extensions: extensionPaths.map((p) => ({ path: p })),
  });

  const jiti = await makeJiti(extensionPaths);
  // （旧的"未找到 jiti"单行提示已并入下面的汇总，避免重复输出。）

  // F19：单个扩展的加载失败要能进 TUI 的 `[Extension issues]`（ref
  // `interactive-mode.ts:1874-1894`）。这里逐条收集 `{path, message}`，随 `ext.ready` 上报；
  // stderr 输出保持原样（调试仍看得到完整栈）。
  const loadErrors = [];
  // F24/S3：缺 jiti 时把"被跳过的 .ts 扩展"合并成**一条**汇总（此前每个扩展一条 stderr + 一条
  // 屏幕条目，10 个扩展就刷 30 行）。ref 没有这个场景，按我们的提示需求实现并登记为偏离。
  const skippedTs = [];
  const summarize = (error) => {
    const text = String(error?.message ?? error ?? "unknown error");
    const first = text.split("\n")[0].trim();
    return first.length > 200 ? `${first.slice(0, 200)}…` : first;
  };

  for (const extensionPath of extensionPaths) {
    if (!jiti && /\.(ts|mts)$/i.test(extensionPath)) {
      skippedTs.push(extensionLabel(extensionPath));
      continue;
    }
    try {
      const module = await loadExtension(extensionPath, jiti);
      // jiti 的 `{ default: true }` 直接返回默认导出（函数）⇒ 先判函数；
      // 原生 ESM 的命名空间则取 `.default` / `.extension`。
      const factory = typeof module === "function" ? module : (module?.default ?? module?.extension);
      if (typeof factory !== "function") {
        process.stderr.write(`[ext-host] ${extensionPath}: no default export factory\n`);
        loadErrors.push({ path: extensionPath, message: "no default export factory" });
        continue;
      }
      currentExtensionPath = extensionPath;
      await factory(pi);
      currentExtensionPath = "";
    } catch (error) {
      process.stderr.write(`[ext-host] ${extensionPath}: ${error?.stack ?? error}\n`);
      // 单个扩展加载失败不拖垮其它扩展（ref 也是逐个报错）。
      loadErrors.push({ path: extensionPath, message: summarize(error) });
    }
  }

  if (skippedTs.length > 0) {
    const names = skippedTs.join(", ");
    // F34/W1d：把**已搜索的全部候选目录**与平台对应的设置示例一起打出来，
    // 否则真机只能看到"未找到 jiti"，无法一次定位（Windows 上路径语法还不同）。
    const searched = searchAttempts.length > 0 ? searchAttempts.map((dir) => `  - ${dir}`).join("\n") : "  （无）";
    const hint = process.platform === "win32"
      ? "set PI_JITI=%LOCALAPPDATA%\\pnpm\\global\\v11\\<hash>\\node_modules\\jiti"
      : "export PI_JITI=/path/to/node_modules/jiti";
    process.stderr.write(
      `[ext-host] 未找到 jiti：${skippedTs.length} 个 .ts 扩展被跳过（${names}）。\n` +
        `已搜索：\n${searched}\n` +
        `安装：npm i -g jiti（或 pnpm i -g jiti）；或设置 PI_JITI 指向 jiti 包目录，例如：\n  ${hint}\n`,
    );
    loadErrors.push({
      path: `${skippedTs.length} 个 .ts 扩展被跳过`,
      message:
        `未找到 jiti。受影响：${names}。已搜索：${searchAttempts.join(" | ") || "（无）"}。` +
        "安装 npm i -g jiti（或 pnpm i -g jiti），或设置 PI_JITI=<jiti 包目录>（Windows 例：set PI_JITI=%LOCALAPPDATA%\\pnpm\\global\\v11\\<hash>\\node_modules\\jiti）。",
    });
  }

  // 同名命令按 ref `runner.ts:741-772` 的规则加 `:N` 后缀生成 invocationName；
  // 菜单显示与 `/cmd` 分发都用它（重名的两个扩展各自可用）。
  const resolvedCommands = (() => {
    const total = new Map();
    for (const command of commands) total.set(command.name, (total.get(command.name) ?? 0) + 1);
    const seen = new Map();
    const taken = new Set();
    return commands.map((command) => {
      const occurrence = (seen.get(command.name) ?? 0) + 1;
      seen.set(command.name, occurrence);
      let invocationName = (total.get(command.name) ?? 0) > 1 ? `${command.name}:${occurrence}` : command.name;
      let suffix = occurrence;
      while (taken.has(invocationName)) {
        suffix += 1;
        invocationName = `${command.name}:${suffix}`;
      }
      taken.add(invocationName);
      return { ...command, invocationName };
    });
  })();
  await send("ext.ready", {
    tools: [...tools.values()].map((tool) => ({
      name: tool.name,
      description: tool.description ?? "",
      parameters: tool.parameters ?? { type: "object", properties: {} },
      promptSnippet: tool.promptSnippet,
      promptGuidelines: tool.promptGuidelines ?? [],
    })),
    commands: resolvedCommands,
    events,
    // 新增可选字段（F19）：旧宿主不认识就忽略，新宿主据此渲染 `[Extension issues]`。
    loadErrors,
    flags: [...flagSpecs.entries()].map(([name, spec]) => ({
      name,
      type: spec.type,
      ...(spec.default !== undefined ? { default: spec.default } : {}),
    })),
    // 注册了几个 Markdown 变换器（宿主据此决定是否每帧走一次 RPC）。
    markdownTransformerCount: markdownTransformers.length,
  });
}

main().catch((error) => {
  console.error(`[ext-host] fatal: ${error?.stack ?? error}`);
  process.exit(1);
});
