#!/usr/bin/env node
// tools/difftest/run.mjs —— 事件驱动的差分测试运行器（先只做 TS 侧，Rust 侧留桩）。
//
// 同步方式：**只用事件，不用墙钟**。场景脚本是一串动作，动作之间靠可观测事件推进：
//   waitEvent{match}   等到 stdout 里出现匹配的事件行
//   send{command}      发一条 rpc 命令，并等到它的 response 回来
//   releaseGate        放行 mock 的闸门（闸门是场景脚本里的显式动作）
//   waitMockConsumed{n}等 mock 报告已完整回放 n 个响应且没有在途连接
//   waitExit / waitIdle 收尾：json/print 等进程退出，rpc 等 agent 结束且 mock 空闲
// 超时只是失败保护，不是正常路径的一部分。
//
// 用法：node tools/difftest/run.mjs <scenario.json> [--runs N] [--out golden]

import { spawn } from "node:child_process";

import { normalize } from "./normalize.mjs";
import { assertSafeOutDir } from "./compare.mjs";
import fs from "node:fs/promises";
import fss from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";

const TS_PI = process.env.TS_PI_ENTRY;
const IMPL = process.env.DIFTEST_IMPL === "pi-rs" ? "pi-rs" : "ts";
const MOCK_BIN = process.env.MOCK_PROVIDER_BIN || "target/debug/mock-provider";
const REPO = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../..");
const TIMEOUT_MS = Number(process.env.DIFTEST_TIMEOUT_MS ?? 60000);
// 最近一次 runOnce 用的 mock 请求日志路径：失败留档时要把它的内容一起留下来
// （判定"abort 是否真的没发出请求"这类问题，只有这份日志能回答）。
let lastRequestLog = null;

const args = process.argv.slice(2);

// **防护**：`DIFTEST_IMPL=pi-rs` 的产出**绝不能**写回 `golden/`——那是 TS 录的基线，
// 让未完成的实现覆盖它等于毁掉参照。默认输出目录只在跑 TS 时才是 `golden/`。
if (IMPL === "pi-rs" && !args.includes("--out")) {
  console.error(
    "[difftest] DIFTEST_IMPL=pi-rs 必须显式给 --out <目录>：pi-rs 的产出不得写回 golden/（基线只能由 TS 录制）",
  );
  process.exit(2);
}

import {
  EventQueue,
  createFile as primitiveCreateFile,
  releaseGate as primitiveReleaseGate,
  send as primitiveSend,
  waitEvent as primitiveWaitEvent,
  waitExit as primitiveWaitExit,
  waitIdle as primitiveWaitIdle,
  waitMockConsumed as primitiveWaitMockConsumed,
  waitState as primitiveWaitState,
} from "./primitives.mjs";

const scenarioPath = path.resolve(args[0] ?? "");
let runs = 3;
let outRoot = "golden";
for (let i = 1; i < args.length; i++) {
  if (args[i] === "--runs") runs = Number(args[i + 1]);
  if (args[i] === "--out") outRoot = args[i + 1];
}
// `--out` 必须是**仓库内的相对路径**：`path.join(REPO, outRoot, id)` 不会因绝对路径重置，
// 传 `/tmp/x` 会把产出写进仓库的 `tmp/x/` 下（真实踩过，见 notes/decisions.md）。
try {
  assertSafeOutDir(outRoot, REPO);
} catch (error) {
  console.error(`[difftest] ${error.message}`);
  process.exit(2);
}

if (!TS_PI && IMPL !== "pi-rs") {
  console.error(
    "必须设置 TS_PI_ENTRY 指向 npm 包里的 dist/bundle/cli.js（跑 pi-rs 时把 DIFTEST_IMPL 设为 pi-rs）",
  );
  process.exit(2);
}

const scenario = JSON.parse(await fs.readFile(scenarioPath, "utf-8"));

// 与 mock 的 `deny_unknown_fields` 对齐：拼错或大小写写错的键直接拒绝，不静默忽略
// （D0：`gateAfterChunks` 一度从未生效就是这么来的）。
const KNOWN_TOP_LEVEL = new Set([
  "id", "description", "provider", "model", "mode", "prompt", "reasoningModel", "cost",
  "contextWindow", "maxTokens", "settings", "commands", "followUp", "responses", "script",
  "files", "globalFiles", "sessionFiles", "cliArgs", "enableSkills", "enablePromptTemplates",
  "expectInjections",
]);
const KNOWN_STEP = new Set([
  "file", "status", "retryAfter", "gateAfterChunks", "disconnectAfterChunks", "chunkByEvent",
]);
const KNOWN_ACTIONS = new Set([
  "send", "createFile", "waitEvent", "waitState", "releaseGate", "waitExit",
]);
{
  const unknownTop = Object.keys(scenario).filter((key) => !KNOWN_TOP_LEVEL.has(key));
  if (unknownTop.length > 0) {
    throw new Error(`场景 ${scenarioPath} 含未知顶层键: ${unknownTop.join(", ")}`);
  }
  for (const [index, step] of (scenario.responses ?? []).entries()) {
    const unknownStep = Object.keys(step).filter((key) => !KNOWN_STEP.has(key));
    if (unknownStep.length > 0) {
      throw new Error(`场景 ${scenarioPath} 的 responses[${index}] 含未知键: ${unknownStep.join(", ")}`);
    }
  }
  for (const step of scenario.script ?? []) {
    if (!KNOWN_ACTIONS.has(step.action)) {
      throw new Error(`场景 ${scenarioPath} 的 script 动作 ${step.action} 未知`);
    }
  }
}
const id = scenario.id;
const provider = scenario.provider;
const model = scenario.model ?? "mock-model";
const mode = scenario.mode ?? "print";
const script = scenario.script ?? [{ action: "waitExit" }];

const withTimeout = (promise, what, ms = TIMEOUT_MS) =>
  Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error(`${what} 超时（${ms}ms），属于失败保护被触发`)), ms)),
  ]);

const waitForLine = (child, prefix) =>
  withTimeout(
    new Promise((resolve, reject) => {
      const rl = readline.createInterface({ input: child.stdout });
      const done = (fn, arg) => {
        rl.close();
        rl.removeAllListeners();
        fn(arg);
      };
      rl.on("line", (line) => {
        if (line.startsWith(prefix)) done(resolve, line.slice(prefix.length).trim());
      });
      child.stdout.on("end", () => done(reject, new Error(`mock 退出前没有打印 ${prefix}`)));
      child.on("exit", () => done(reject, new Error(`mock 退出前没有打印 ${prefix}`)));
    }),
    `等待 mock 打印 ${prefix}`,
  );

/** 确定性地回收一个子进程：先给时间优雅退出，再 TERM，最后 KILL。 */
async function reapChild(child, label, graceMs = 5000) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolve) => child.once("exit", resolve));
  const grace = new Promise((resolve) => setTimeout(resolve, graceMs));
  await Promise.race([exited, grace]);
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, graceMs))]);
  if (child.exitCode !== null || child.signalCode !== null) return;
  console.error(`[difftest] ${label} 未响应 SIGTERM，改用 SIGKILL`);
  child.kill("SIGKILL");
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, graceMs))]);
}

/**
 * pi-rs 的命令行参数。
 *
 * 与 TS 侧的差别：只传本项目**真的实现**的开关——TS 侧那些 `--no-extensions`/`--no-themes`
 * 等会被 pi-rs 当作不支持而**报错退出**（这是有意的：不静默忽略）。
 */
/** 由 TS 入口推导包目录（`<pkg>/dist/bundle/cli.js` → `<pkg>`）。 */
function packageDirOf(tsPi) {
  if (!tsPi) return "";
  return path.dirname(path.dirname(path.dirname(tsPi)));
}

function piRsArgs(mode, model, sess, scenario) {
  const args = [
    "--mode",
    mode === "rpc" ? "rpc" : "json",
    "--provider",
    "mock",
    "--model",
    model,
    "--api-key",
    "mock-key",
    "--session-dir",
    sess,
  ];
  for (const extra of scenario.cliArgs ?? []) {
    if (PI_RS_UNSUPPORTED.has(extra)) continue;
    args.push(extra);
  }
  if (mode !== "rpc") args.push(scenario.prompt ?? "say hi");
  return args;
}

/** TS 专用、pi-rs 会拒绝的开关。 */
const PI_RS_UNSUPPORTED = new Set([
  "--no-extensions",
  "-ne",
  "--no-themes",
  "--no-skills",
  "-ns",
  "--no-prompt-templates",
  "-np",
  "--no-context-files",
  "-nc",
  "--offline",
]);

const writeModelsJson = async (cfgDir, baseUrl, scenario, port) => {
  const models = {
    providers: {
      mock: {
        name: "Mock",
        baseUrl,
        api: provider,
        models: [
          {
            id: scenario.model ?? "mock-model",
            name: "Mock Model",
            reasoning: scenario.reasoningModel ?? provider === "anthropic-messages",
            input: ["text", "image"],
            cost: scenario.cost ?? { input: 1, output: 2, cacheRead: 0, cacheWrite: 0 },
            contextWindow: scenario.contextWindow ?? 200000,
            maxTokens: scenario.maxTokens ?? 4096,
          },
        ],
      },
    },
  };
  await fs.mkdir(cfgDir, { recursive: true });
  await fs.writeFile(path.join(cfgDir, "models.json"), `${JSON.stringify(models, null, 2)}\n`);
  const settings = scenario.settings ?? {};
  await fs.writeFile(path.join(cfgDir, "settings.json"), `${JSON.stringify(settings, null, 2)}\n`);
  await fs.writeFile(
    path.join(cfgDir, "auth.json"),
    `${JSON.stringify({ mock: { type: "api_key", key: "mock-key" } }, null, 2)}\n`,
  );
  return port;
};

const snapshotTree = async (dir) => {
  const entries = [];
  const walk = async (current, prefix) => {
    const items = await fs.readdir(current, { withFileTypes: true });
    for (const item of items.sort((a, b) => a.name.localeCompare(b.name))) {
      const rel = prefix ? `${prefix}/${item.name}` : item.name;
      const full = path.join(current, item.name);
      if (item.isDirectory()) await walk(full, rel);
      else {
        const stat = await fs.stat(full);
        entries.push({ path: rel, size: stat.size > 4096 ? "<BIG>" : stat.size });
      }
    }
  };
  await walk(dir, "");
  return entries;
};

/** 事件流读取器：行到达即入队，等待方按 match 逐条消费。 */
export 

const mockStatus = async (port) => {
  const res = await fetch(`http://127.0.0.1:${port}/control/status`);
  return res.json();
};

const runOnce = async () => {
  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), `difftest-${id}-`));
  const home = path.join(tmp, "home");
  const cfg = path.join(tmp, "cfg");
  const sess = path.join(tmp, "sess");
  const work = path.join(tmp, "work");
  for (const dir of [home, cfg, sess, work]) await fs.mkdir(dir, { recursive: true });
  for (const [rel, content] of Object.entries(scenario.files ?? {})) {
    const target = path.join(work, rel);
    await fs.mkdir(path.dirname(target), { recursive: true });
    // 内容以 fixtures/ 或 golden/ 开头时按路径复制（用于二进制夹具，如 png）。
    if (typeof content === "string" && /^(fixtures|golden)\//.test(content)) {
      await fs.copyFile(path.join(REPO, content), target);
      continue;
    }
    await fs.writeFile(target, content);
  }
  // globalFiles 写进配置目录（模拟全局 AGENTS.md / CLAUDE.md）。
  for (const [rel, content] of Object.entries(scenario.globalFiles ?? {})) {
    const target = path.join(cfg, rel);
    await fs.mkdir(path.dirname(target), { recursive: true });
    await fs.writeFile(target, content);
  }
  if (scenario.sessionFiles) {
    // 构造输入会话：把脚本给的会话文件原样放进会话目录。
    // 会话头里的 cwd 必须与运行目录一致，pi 才会把这份会话当成当前项目的会话；
    // 因此支持在内容里写 <CWD> 占位符，由运行器替换成真实的临时工作目录。
    const seeded = scenario.sessionFiles.content.replaceAll("<CWD>", work);
    await fs.writeFile(path.join(sess, scenario.sessionFiles.fileName), seeded);
  }

  const requestLog = path.join(tmp, "requests.jsonl");
  lastRequestLog = requestLog;
  const mock = spawn(MOCK_BIN, [scenarioPath, "--port", "0", "--request-log", requestLog], {
    cwd: REPO,
    stdio: ["ignore", "pipe", "pipe"],
  });
  mock.stderr.on("data", (d) => process.stderr.write(`[mock] ${d}`));
  const addr = await waitForLine(mock, "listening ");
  const port = Number(addr.split(":").pop());
  await writeModelsJson(cfg, `http://127.0.0.1:${port}/v1`, scenario, port);

  // 被测实现：默认 TS（黄金输出来源），`DIFTEST_IMPL=pi-rs` 时跑 Rust 侧。
  const impl = IMPL;
  const cliArgs = impl === "pi-rs" ? piRsArgs(mode, model, sess, scenario) : [
    TS_PI,
    "--mode",
    mode === "rpc" ? "rpc" : "json",
    "--provider",
    "mock",
    "--model",
    model,
    "--api-key",
    "mock-key",
    "--offline",
    "--no-extensions",
    "--no-themes",
    ...(scenario.enablePromptTemplates ? [] : ["--no-prompt-templates"]),
    ...(scenario.enableSkills ? [] : ["--no-skills"]),
    "--session-dir",
    sess,
  ];
  for (const extra of scenario.cliArgs ?? []) cliArgs.push(extra);
  // pi-rs 分支已经把 prompt 作为位置参数放进 `piRsArgs` 了；这里只给 TS 加 `-p`。
  // 两边都加会让 pi-rs 收到 `-p <prompt>`，而 `-p` 会**吃掉**紧跟的位置参数 →
  // prompt 变成两份（实测请求体里出现 `"say hi say hi"`）。这条是 runner 侧的缺陷。
  if (mode !== "rpc" && impl === "ts") cliArgs.push("-p", scenario.prompt ?? "say hi");

  const child = spawn(
    impl === "pi-rs" ? path.join(REPO, "target/debug/pi-rs") : process.execPath,
    cliArgs,
    {
    cwd: work,
    env: {
      ...process.env,
      HOME: home,
      PI_CODING_AGENT_DIR: cfg,
      // 系统提示的 docs 段内嵌**包目录的绝对路径**（golden 记的就是绝对路径）。
      // TS 自己由 `getPackageDir()` 推导；pi-rs 没有对应目录，因此这里显式把 TS 包目录
      // 传给两边，使 C 层的这一行可比。**仅用于差分**：生产环境 pi-rs 应指向自己的包目录。
      PI_PACKAGE_DIR: packageDirOf(TS_PI),
      PI_CODING_AGENT_SESSION_DIR: sess,
      PI_OFFLINE: "1",
      PI_TELEMETRY: "0",
      NO_COLOR: "1",
    },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );

  if (mode !== "rpc") {
    // print/json 模式靠 stdin EOF 结束，否则会一直等输入。
    child.stdin.end();
  }


  const queue = new EventQueue();
  let settledCount = 0;
  // D1：mock 的注入计数器（闸门 / 断连 / Retry-After），收尾时抓一次。
  let injections = null;
  let stdout = "";
  let stderr = "";
  let exitCodeOverride = null;
  // 轮询（waitState）期间，get_state 的中间响应先缓冲、不进事件流：
  // 轮询次数取决于时序，属于采集行为而非被测行为。
  // 场景脚本里用了 waitState 就整体屏蔽 get_state 响应：轮询次数与迟到的响应都取决于时序，
  // 只有命中值的那条由 waitState 自己补进事件流。
  // print 模式场景可能没有 script；此时不做轮询屏蔽。
  let polling = (scenario.script ?? []).some((s) => s.action === "waitState") ? [] : null;
  const rl = readline.createInterface({ input: child.stdout });
  rl.on("line", (line) => {
    if (polling !== null && line.includes('"command":"get_state"')) {
      polling.push(line);
      queue.push(line);
      return;
    }
    stdout += `${line}\n`;
    if (line.includes('"type":"agent_settled"')) settledCount += 1;
    queue.push(line);
  });
  child.stderr.on("data", (d) => (stderr += d));
  const exited = new Promise((resolve) => child.on("close", resolve));
  // 契约（RUNBOOK-4 / P3.-2）：等待只能以"条件满足 / 超时失败 / 子进程提前退出失败"结束。
  // 失败原因里带上**子进程 stderr**：pi-rs 启动即退出（比如配置/会话读取失败）时，
  // 只说"输出流已关闭"会让人完全看不到原因，而 stderr 是唯一线索。
  const withStderr = (what) =>
    stderr.trim() ? `${what}（子进程 stderr：\n${stderr.trim()}\n）` : what;
  child.on("exit", () => queue.failAll(withStderr("子进程已退出")));
  rl.on("close", () => queue.failAll(withStderr("子进程输出流已关闭")));

  const sendCommand = (command) => {
    // 命令参数里也支持路径占位符：`<CWD>`（工作目录）与 `<SESSION_DIR>`（会话目录）。
    // 之所以需要：`switch_session` 要传**绝对路径**、`new_session.parentSession` 也是路径，
    // 而运行目录是临时创建的，场景文件里写不出真实路径。（只替换这两个安全占位符。）
    const line = JSON.stringify(command)
      .replaceAll("<CWD>", work)
      .replaceAll("<SESSION_DIR>", sess);
    child.stdin.write(`${line}\n`);
    return queue.take(`"command":"${command.type}"`, `等待 ${command.type} 的响应`);
  };

  // 场景的初始命令先发，逐条等到响应确认，再按 script 推进。
  for (const command of scenario.commands ?? []) await sendCommand(command);

  // 步骤追踪（RUNBOOK-4 / P3.-2）：每个脚本步骤记录起止、结果与匹配到的事件序号。
  // 没有它，"等待原语这次是否真的匹配上了"就只能靠猜——Q14 正是这么漏过去的。
  const stepTrace = [];
  let eventSeq = 0;
  await withTimeout((async () => {
  for (const [stepIndex, step] of script.entries()) {
    const startedAt = Date.now();
    const record = (result, detail) =>
      stepTrace.push({
        index: stepIndex,
        action: step.action,
        ms: Date.now() - startedAt,
        result,
        ...(detail ? { detail } : {}),
      });
    try {
    switch (step.action) {
      case "waitEvent": {
        const line = await primitiveWaitEvent({ queue, match: step.match });
        eventSeq += 1;
        record("matched", `命中第 ${eventSeq} 条事件: ${line.slice(0, 120)}`);
        break;
      }
      case "send":
        await primitiveSend({ queue, child, command: step.command });
        break;
      case "waitState": {
        // 轮询次数与迟到响应都不该进 B 层：只把命中的那条补进事件流。
        const matched = await primitiveWaitState({
          queue,
          child,
          want: step.match ?? "true",
        });
        stdout += `${matched}\n`;
        if (matched.includes('"type":"agent_settled"')) settledCount += 1;
        break;
      }
      case "createFile":
        // 时序外部化：工具执行阻塞在标志文件上，命令确认后才放行。
        await primitiveCreateFile({
          fsImpl: fs,
          dir: work,
          path: step.path,
          content: step.content,
        });
        break;
      case "releaseGate":
        await primitiveReleaseGate({ fetchImpl: fetch, port });
        break;
      case "waitMockConsumed":
        await primitiveWaitMockConsumed({ mockStatus: () => mockStatus(port), n: step.n, sleep: (ms) => new Promise((r) => setTimeout(r, ms)) });
        break;
      case "waitIdle":
        // fresh：忽略已到达但未消费的旧事件，避免匹配到上一轮的 agent_end。
        await primitiveWaitIdle({ queue, fresh: step.fresh });
        break;
      case "waitExit":
        // rpc 模式常驻运行，退出由脚本之后的统一收尾负责；这里只等进程真的结束（print/json）。
        if (mode !== "rpc") await withTimeout(exited, "等待进程退出");
        break;
      default:
        throw new Error(`未知动作: ${step.action}`);
    }
    } catch (error) {
      record("failed", error instanceof Error ? error.message : String(error));
      throw error;
    }
    if (!stepTrace.some((entry) => entry.index === stepIndex)) record("ok");
  }
  })(), `场景脚本 ${id}`);
  console.log(`[difftest] ${id} stepTrace: ${JSON.stringify(stepTrace)}`);

  if (mode === "rpc") {
    // 脚本跑完，确认 mock 已消费完脚本声明的全部响应再收尾。
    // 收尾判据（rpc）：先等 agent 报告本轮已 settle（`agent_settled` 事件），
    // 再要求 mock 没有在途连接。只看 inFlight / served 都不够——请求还没发出时
    // inFlight 就是 0，会导致提前关闭 stdin、prompt 永远不产生 assistant 消息。
    // expectRuns：本次运行预期产生几个 agent run（每个 run 结束时会有一条 agent_settled）。
    // 场景没写就按 1 处理；写了 expectSettled:false 的场景直接跳过。
    const wantRuns = scenario.expectRuns ?? 1;
    if (scenario.expectSettled !== false && wantRuns > 0) {
      await withTimeout(
        (async () => {
          while (settledCount < wantRuns) {
            await queue.take('"type":"agent_settled"', "等待 agent_settled");
          }
        })(),
        `等待 ${wantRuns} 个 agent_settled`,
        20000,
      );
    }
    // 收尾判据：脚本声明的响应都已被 mock 服务过，且当前没有在途连接。
    // 只看 inFlight 是不够的——请求还没发出时它就已经是 0，会导致提前关闭 stdin、
    // prompt 永远不产��� assistant 消息（表现为"第 2 次起 session 为空"）。
    // run 结束（agent_settled）已经保证会话跑完，这里只要求 mock 回到空闲；
    // "服务了几个"不作硬判据——场景不一定会消费脚本里的全部响应（如 abort 后的尾随响应）。
    try {
      await withTimeout(
        (async () => {
          for (;;) {
            const status = await mockStatus(port);
            if (status.inFlight === 0) return;
            await new Promise((r) => setTimeout(r, 5));
          }
        })(),
        "等待 mock 回到空闲",
        10000,
      );
    } catch {
      // 超时只作失败保护；run 已 settle，继续收尾。
    }
    child.stdin.end();
    await withTimeout(exited, "等待 rpc 进程退出");
  }
  // followUp：同一批临时目录里再跑一次（--continue / --resume），请求日志会累加。
  for (const extra of scenario.followUp ?? []) {
    // **分阶段换实现**（X01/X02 用）：`extra.impl === "other"` 表示"与主实现相反的那个"。
    // 这样一个场景文件既能录 TS 基线（主=TS → other=pi-rs… 不，录制时必须两边都 TS），
    // 也能用于对拍。注意：`"other"` 在录制（DIFTEST_IMPL 未设，主 = TS）时就是 pi-rs，
    // 所以**录制跨实现场景时必须显式写 `impl: "ts"`**，对拍时写 `impl: "other"`。
    const extraImpl = (() => {
      // **录制跨实现场景的基线**（X01/X02）时用 `DIFTEST_FORCE_TS=1`：所有阶段都跑 TS，
      // 于是 golden 是 TS→TS→TS，而对拍时（不设该变量）按场景里写的绝对实现跑，
      // 例如 X01 = TS（主）→ pi-rs → TS。比较的就成了"Rust 的续写与 TS 的续写能否区分"。
      if (process.env.DIFTEST_FORCE_TS === "1") return "ts";
      // **默认与主实现相同**：`followUp` 是"同一场景的后续阶段"，本来就该跟着主实现走。
      // 我第一版默认 `"ts"`，于是 `DIFTEST_IMPL=pi-rs` 的对拍里**第二阶段偷偷跑的是 TS**——
      // S14 的差分因此显示成"续写阶段多了一条 system"，其实那条 system 是 TS 写的，pi-rs 根本没跑。
      const want = extra.impl ?? "same";
      if (want === "same") return IMPL;
      if (want === "ts") return "ts";
      if (want === "pi-rs") return "pi-rs";
      if (want === "other") return IMPL === "ts" ? "pi-rs" : "ts";
      throw new Error(`followUp.impl 只接受 ts / pi-rs / other，收到 ${want}`);
    })();
    const extraArgs = extraImpl === "pi-rs" ? [
      ...piRsArgs(mode, model, sess, {
        ...scenario,
        prompt: extra.prompt ?? "follow up",
      }),
      // **`extra.cliArgs` 必须带上**：我第一版漏了它，于是 `--continue` 从未传给 pi-rs，
      // 第二阶段另起了一个新会话（会话里出现第二条 `session`）——却被"TS 在跑第二阶段"
      // 掩盖成了"续写已可用"。
      ...(extra.cliArgs ?? []),
    ] : [
      TS_PI,
      "--mode",
      mode === "rpc" ? "rpc" : "json",
      "--provider",
      "mock",
      "--model",
      model,
      "--api-key",
      "mock-key",
      "--offline",
      "--no-extensions",
      "--no-themes",
      "--no-prompt-templates",
      "--no-skills",
      "--session-dir",
      sess,
      ...(extra.cliArgs ?? []),
      "-p",
      extra.prompt ?? "follow up",
    ];
    const extraProgram =
      extraImpl === "pi-rs" ? path.join(REPO, "target/debug/pi-rs") : process.execPath;
    const extraChild = spawn(extraProgram, extraArgs, {
      cwd: work,
      env: {
        ...process.env,
        HOME: home,
        PI_CODING_AGENT_DIR: cfg,
        PI_CODING_AGENT_SESSION_DIR: sess,
        PI_OFFLINE: "1",
        PI_TELEMETRY: "0",
        NO_COLOR: "1",
        // **后续阶段也要同一个 `PI_PACKAGE_DIR`**：docs 段的三个路径由它决定。
        // 漏了它时续写阶段的系统提示会变成 `/README.md` 这种空包路径——
        // S14 的 C 层第 2 个请求就是这样暴露的（两侧长度 6209 vs 5996）。
        PI_PACKAGE_DIR: packageDirOf(TS_PI),
      },
      stdio: ["pipe", "pipe", "pipe"],
    });
    let extraOut = "";
    extraChild.stdout.on("data", (d) => (extraOut += d));
    extraChild.stderr.on("data", (d) => (stderr += d));
    extraChild.stdin.end();
    const extraCode = await withTimeout(
      new Promise((resolve) => extraChild.on("close", resolve)),
      "等待 followUp 运行结束",
    );
    stdout += `\n--- followUp (${extra.cliArgs?.join(" ") ?? ""}) ---\n${extraOut}`;
    if (extraCode !== 0) exitCodeOverride = extraCode;
  }

  // D1：抓注入计数（所有 mode 都要抓，print 模式也要）。
  // 计数器是"最终一致"的：闸门/断连可能发生在 pi 已经断开之后，因此轮询到期望值
  // 齐了再抓，最多等 2 秒。
  {
    const want = scenario.expectInjections ?? {};
    const deadline = Date.now() + 2000;
    for (;;) {
      try {
        injections = await mockStatus(port);
      } catch {
        injections = null;
        break;
      }
      const settled = Object.entries(want).every(([key, target]) => (injections[key] ?? 0) >= target);
      if (settled || Date.now() >= deadline) break;
      await new Promise((r) => setTimeout(r, 20));
    }
  }
  await reapChild(child, `${id} 的 pi 进程`);
  await reapChild(mock, `${id} 的 mock 进程`);

  const requests = fss.existsSync(requestLog) ? await fs.readFile(requestLog, "utf-8") : "";
  const ctx = { tmp, home, port };
  const sessionFiles = fss.existsSync(sess) ? await fs.readdir(sess) : [];
  let session = "";
  for (const file of sessionFiles.filter((f) => f.endsWith(".jsonl")).sort()) {
    session += await fs.readFile(path.join(sess, file), "utf-8");
  }
  const exitCode = child.exitCode;

  const result = {
    events: normalize(stdout, ctx),
    stepTrace,
    injections: injections ?? null,
    requests: normalize(requests, ctx),
    session: normalize(session, ctx),
    fs: await snapshotTree(work),
    stderr: normalize(stderr, ctx),
    exitCode: exitCodeOverride ?? exitCode,
  };
  await fs.rm(tmp, { recursive: true, force: true });
  return result;
};

/**
 * 产出是否有效的门槛。
 *
 * 只看"三次是否一致"不够：一次"一致地失败"的运行（例如工具根本没被调用、session 空）
 * 也会被判成稳定基线。这里显式检查产物本身是否成形。
 */
const sanityCheck = (scenario, result) => {
  const problems = [];
  // D1：注入断言。场景可声明期望的注入次数，不符即判失败——
  // 避免"开关写了但从未生效"这类静默失效再次发生。
  const expect = scenario.expectInjections;
  if (expect) {
    const actual = result.injections ?? {};
    for (const [key, want] of Object.entries(expect)) {
      const got = actual[key] ?? 0;
      if (got !== want) {
        problems.push(`注入断言失败：${key} 期望 ${want}，实际 ${got}`);
      }
    }
  }
  if (!result.session.trim()) problems.push("session 为空");
  const assistantCount = (result.session.match(/"role":"assistant"/g) ?? []).length;
  if (!scenario.expectNoAssistant && assistantCount === 0) {
    problems.push("session 里没有 assistant 消息");
  }
  const wantsTool = (scenario.responses ?? []).some((r) =>
    /tool_calls|"type":"tool_use"/.test(r.file ?? ""),
  );
  if (wantsTool && !result.events.includes('"type":"tool_execution_end"')) {
    problems.push("场景声明了工具调用，但没有 tool_execution_end");
  }
  return problems;
};

const layerOf = (r) => ({
  // A 层：会话语义内容 + 文件系统副作用 + 退出码
  A: JSON.stringify([r.session, r.fs, r.exitCode]),
  // B 层：--mode json 事件流的类型与顺序
  B: JSON.stringify(r.events),
  // C 层：发给 provider 的请求体
  C: JSON.stringify(r.requests),
});

const main = async () => {
  const results = [];
  const sanityProblems = new Set();
  for (let i = 0; i < runs; i++) {
    let result;
    try {
      result = await runOnce();
    } catch (error) {
      // **失败也要留下产出目录**：否则 P4.9 的"每个场景在结果表里恰好一行"对账会因为
      // "目录不存在"而失败，把真正的原因（脚本等待超时等）掩盖掉。这里写明原因，
      // 然后**照旧抛出**——退出码仍非零，失败依然可见、不被吞掉。
      const failureDir = path.join(REPO, outRoot, id);
      await fs.mkdir(failureDir, { recursive: true });
      for (const name of ["events.jsonl", "requests.jsonl", "session.jsonl"]) {
        await fs.writeFile(path.join(failureDir, name), "");
      }
      await fs.writeFile(path.join(failureDir, "fs.json"), "{}\n");
      await fs.writeFile(
        path.join(failureDir, "meta.json"),
        `${JSON.stringify(
          {
            id,
            provider,
            model,
            mode,
            failed: true,
            failedRun: i + 1,
            reason: String(error?.message ?? error),
          },
          null,
          2,
        )}\n`,
      );
      if (lastRequestLog) {
        try {
          const mockLog = await fs.readFile(lastRequestLog, "utf8");
          await fs.writeFile(path.join(failureDir, "mock-requests.jsonl"), mockLog);
        } catch (logError) {
          await fs.writeFile(
            path.join(failureDir, "mock-requests.jsonl"),
            `（读 mock 请求日志失败：${String(logError?.message ?? logError)}）\n`,
          );
        }
      }
      await fs.writeFile(
        path.join(failureDir, "FAILED.md"),
        `# ${id} 运行失败\n\n第 ${i + 1} 次运行失败。原文：\n\n\`\`\`\n${String(
          error?.stack ?? error,
        )}\n\`\`\`\n\n产出为空——**这是失败，不是"无差异"**。\n`,
      );
      throw error;
    }
    for (const problem of sanityCheck(scenario, result)) {
      sanityProblems.add(`第 ${i + 1} 次: ${problem}`);
    }
    // 每次 run 的原始产物都留一份：A4 的变体集收集与排障都需要按 run 查看。
    const runDir = path.join(REPO, outRoot, "_runs", id, `run-${i + 1}`);
    await fs.mkdir(runDir, { recursive: true });
    await fs.writeFile(path.join(runDir, "events.jsonl"), result.events);
    await fs.writeFile(path.join(runDir, "session.jsonl"), result.session);
    await fs.writeFile(path.join(runDir, "requests.jsonl"), result.requests);
    await fs.writeFile(path.join(runDir, "fs.json"), `${JSON.stringify(result.fs, null, 2)}\n`);
    results.push(result);
    console.log(`[difftest] ${id} 第 ${i + 1}/${runs} 次完成`);
  }
  const layers = results.map(layerOf);
  const sane = sanityProblems.size === 0;
  const stableLayers = sane
    ? {
        A: layers.every((l) => l.A === layers[0].A),
        B: layers.every((l) => l.B === layers[0].B),
        C: layers.every((l) => l.C === layers[0].C),
      }
    : { A: false, B: false, C: false };
  const outDir = path.join(REPO, outRoot, id);
  await fs.mkdir(outDir, { recursive: true });
  await fs.writeFile(path.join(outDir, "events.jsonl"), results[0].events);
  await fs.writeFile(path.join(outDir, "requests.jsonl"), results[0].requests);
  await fs.writeFile(path.join(outDir, "session.jsonl"), results[0].session);
  await fs.writeFile(path.join(outDir, "fs.json"), `${JSON.stringify(results[0].fs, null, 2)}\n`);
  await fs.writeFile(
    path.join(outDir, "meta.json"),
    `${JSON.stringify(
      {
        id,
        provider,
        model,
        mode,
        exitCode: results[0].exitCode,
        runs,
        sane,
        ...(sane ? {} : { sanityProblems: [...sanityProblems] }),
        stableLayers,
        ...(stableLayers.A ? {} : { baseline: "C-only" }),
        stderr: results[0].stderr,
        stepTrace: results[0].stepTrace ?? [],
      },
      null,
      2,
    )}\n`,
  );
  console.log(
    `[difftest] ${id}: sane=${sane} stableLayers=${JSON.stringify(stableLayers)} exit=${results[0].exitCode}` +
      (sane ? "" : ` | 产出未成形: ${[...sanityProblems].join("; ")}`),
  );
  if (!sane) process.exitCode = 2;
};

await main();