// pi-rs 扩展宿主：Node 侧 runner（P9-1-B 第一批）。
//
// 与宿主的契约见 notes/p9-1/protocol.md（S5 决策：loopback TCP + 一次性令牌）。
// 本批只做：握手 → 加载扩展（.mjs/.js）→ 收集 registerTool → 服务 ext.tool.invoke。
// 事件分发 / UI 帧 / 渲染钩子留给后续批次（收到未知方法回 -32601）。

import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { spawnSync } from "node:child_process";
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
let hostCtx = {};
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
function contextForExtension() {
  return {
    cwd: hostCtx.cwd,
    mode: hostCtx.mode,
    hasUI: hostCtx.hasUI,
    model: hostCtx.model,
    thinkingLevel: hostCtx.thinkingLevel,
    // 终端宽度：扩展的页脚/组件按 `render(width)` 排版（ref `footer.ts` 传的是当前列数）。
    width: typeof hostCtx.width === "number" ? hostCtx.width : 80,
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
        let cursor = fromId ?? entries[entries.length - 1]?.id;
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
      // ref `session-manager.ts:1402/1503/1156/1152`。
      getLeafId() {
        const entries = this.getEntries().filter((entry) => entry?.type !== "session");
        return entries[entries.length - 1]?.id ?? null;
      },
      getSessionDir() {
        const file = hostCtx.sessionFile;
        return file ? path.dirname(file) : undefined;
      },
      getEntries() {
        const file = hostCtx.sessionFile;
        if (!file) return [];
        try {
          return fs
            .readFileSync(file, "utf8")
            .split("\n")
            .filter((line) => line.trim().length > 0)
            .map((line) => JSON.parse(line));
        } catch {
          return [];
        }
      },
    },
    // ref `types.ts:294-300` 的 `ContextUsage`；宿主在每帧的 ctx 里带过来。
    getContextUsage() {
      return hostCtx.usage ?? undefined;
    },
    ui: {
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
      setStatus: (key, text) => send("host.ui.setStatus", { key: String(key), text }),
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
        const component = factory({ requestRender() {} }, makeThemeStub(), footerDataStub());
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
}

async function dispatch(frame) {
  const { id, method, params } = frame;
  if (params?.ctx) hostCtx = params.ctx;
  if (method === "ext.tool.invoke") {
    const tool = tools.get(params?.toolName);
    if (!tool) {
      respond(id, undefined, { code: -32601, message: `unknown tool ${params?.toolName}` });
      return;
    }
    try {
      if (params?.ctx) hostCtx = params.ctx;
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
  if (method === "ext.render") {
    // 渲染钩子降级（ref `tool-execution.ts:26-33`）：跑扩展的 renderCall/renderResult，
    // 把返回的组件序列化成帧；不支持的组件/抛错 ⇒ `unsupported`（宿主回落内置渲染）。
    const toolName = params?.toolName;
    const definition = tools.get(toolName);
    const phase = params?.phase === "result" ? "result" : "call";
    const renderer = definition?.[phase === "call" ? "renderCall" : "renderResult"];
    if (typeof renderer !== "function") {
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
    if (params?.ctx) hostCtx = params.ctx;
    const list = [...(handlers.get(name) ?? [])];
    const errors = [];
    // 有返回值语义的两类事件（ref `runner.ts:1136-1153` 的 emitToolCall、
    // `runner.ts:1080-1135` 的 emitToolResult）：
    // - tool_call：按顺序跑，**第一个 block 立刻返回**（后续处理器不再执行）；
    // - tool_result：链式——每个处理器看到前一个改过的 event；某个处理器抛错只记录。
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
          } else if (result) {
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
    // `before_agent_start`：收集 `message`，`systemPrompt` 覆盖后者胜（ref `runner.ts:1312-1365`）。
    if (name === "before_agent_start") {
      const messages = [];
      let systemPrompt;
      for (const handler of list) {
        try {
          const result = await handler({ ...(params?.payload ?? {}) }, contextForExtension());
          if (result?.message) messages.push(result.message);
          if (result?.systemPrompt !== undefined) systemPrompt = result.systemPrompt;
        } catch (error) {
          errors.push(String(error && error.message ? error.message : error));
        }
      }
      respond(id, {
        handled: list.length,
        errors,
        result: { messages, ...(systemPrompt !== undefined ? { systemPrompt } : {}) },
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
    for (const handler of list) {
      try {
        await handler(params?.payload ?? {}, contextForExtension());
      } catch (error) {
        errors.push(String(error && error.message ? error.message : error));
      }
    }
    if (errors.length > 0) {
      process.stderr.write(`[ext-host] event ${name}: ${errors.length} handler(s) failed: ${errors.join("; ")}\n`);
    }
    respond(id, { handled: list.length, errors });
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
  registerMarkdownTransformer(transformer) {
    void transformer;
    // 记录但不应用（首版不渲染扩展的 markdown 变换）。
  },
  registerProvider() {
    noteNotImplemented("pi.registerProvider");
  },
  unregisterProvider() {},
  setSessionName() {},
  getSessionName() {
    return undefined;
  },
  setLabel() {},
  // ref `types.ts:1509`：当前会话名。pi-rs 尚未实现会话命名（无 `/rename`）⇒ 恒 undefined
  // （不是假值：宿主本来就没有名字可给）。
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
  getAllTools() {
    return [...tools.values()].map((tool) => ({
      name: tool.name,
      description: tool.description ?? "",
      parameters: tool.parameters ?? { type: "object", properties: {} },
      promptGuidelines: tool.promptGuidelines ?? [],
      sourceInfo: { kind: "extension" },
    }));
  },
  setActiveTools() {},
  getCommands() {
    return commands.map((command) => ({ name: command.name, description: command.description, source: "extension" }));
  },
  async setModel() {
    noteNotImplemented("pi.setModel");
    return false;
  },
  getThinkingLevel() {
    return "off";
  },
  setThinkingLevel() {},
  events: {
    publish() {},
    on() {
      return () => {};
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
    getExtensionStatuses: () => new Map(),
    // ref `footer.ts` 的 `footerData.getAvailableProviderCount()`：>1 时右侧加 `(provider)` 前缀。
    getAvailableProviderCount: () =>
      typeof hostCtx.availableProviderCount === "number" ? hostCtx.availableProviderCount : 1,
    getGitBranch: () => undefined,
    getCwd: () => hostCtx.cwd,
    onBranchChange: () => () => {},
  };
}

function makeThemeStub() {
  return {
    fg: (color, text) => {
      const ansi = shimTheme.FG_COLORS[color];
      if (!ansi) throw new Error(`Unknown theme color: ${color}`);
      return `${ansi}${String(text ?? "")}${shimTheme.RESET_FG}`;
    },
    bg: (_color, text) => String(text ?? ""),
    bold: (text) => String(text ?? ""),
    dim: (text) => String(text ?? ""),
    italic: (text) => String(text ?? ""),
  };
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
  });
}

main().catch((error) => {
  console.error(`[ext-host] fatal: ${error?.stack ?? error}`);
  process.exit(1);
});
