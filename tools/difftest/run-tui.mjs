// TUI 场景运行器（P7-2 W2）：驱动 pty-helper.py（unshare -n 内）、临时目录、PTY 快照与真实退出码。
// 步骤映射：waitScreen→helper waitFor；waitStable/snapshot→helper waitStable；type→helper text。
// 用法：node tools/difftest/run-tui.mjs --scenario <path> --out <dir> [--impl ts|pi-rs]
import { spawn } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";

const REPO = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../..");
const HELPER = path.join(REPO, "tools/difftest/pty-helper.py");

export function stepsToHelperScript(steps) {
  const script = [];
  for (const step of steps ?? []) {
    if (step.action === "waitScreen") {
      script.push({ waitFor: { contains: step.contains, timeoutMs: step.timeoutMs ?? 10000 } });
    } else if (step.action === "waitStable" || step.action === "snapshot") {
      script.push({ waitStable: { quietMs: step.quietMs ?? 300, timeoutMs: step.timeoutMs ?? 10000 } });
      if (step.action === "snapshot") {
        // P7-4/W0e：快照点同时打一个 mark，供比较时把 raw 截到"会话进行中"的位置。
        script.push({ mark: step.name ?? "snapshot" });
      }
    } else if (step.action === "type") {
      script.push(step.delayMs === undefined ? { text: step.text } : { delayMs: step.delayMs, text: step.text });
    } else if (step.action === "waitStrip") {
      // 去转义后的文本等待（helper 的 waitStrip）：TUI 回复/用量文本在原始流里会被 SGR 切分。
      script.push({ waitStrip: { contains: step.contains, timeoutMs: step.timeoutMs ?? 10000 } });
    } else if (step.action === "delay") {
      // 显式等待：TUI 在等模型回复时**无输出** ⇒ `waitStable` 会过早判稳；用固定等待补足。
      script.push({ delayMs: step.ms ?? step.delayMs ?? 1000 });
    }
  }
  return script;
}

async function main() {
  const argv = process.argv.slice(2);
  const get = (n, d) => { const i = argv.indexOf(n); return i >= 0 ? argv[i + 1] : d; };
  // P7-6s/W2：**让 `--impl` 生效**（此前只读环境变量 `DIFTEST_IMPL`，`--impl` 被静默忽略，
  // 导致"以为是 pi-rs、实际录到 TS"）。规则：
  //   - 未知参数（不认识的 `--xxx`）⇒ 报错退出 2（不再静默忽略）；
  //   - `--impl` 值必须是 ts / pi-rs，否则退出 2；
  //   - `--impl` 与 `DIFTEST_IMPL` 同时给出且不同 ⇒ 退出 2（避免歧义）。
  const KNOWN = new Set(["--scenario", "--out", "--impl"]);
  const unknown = [];
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (!arg.startsWith("--")) continue; // 值
    if (!KNOWN.has(arg)) unknown.push(arg);
    else i += 1; // 跳过其值
  }
  if (unknown.length > 0) {
    console.error(`run-tui: 未知参数 ${unknown.join(" ")}（支持 --scenario/--out/--impl）`);
    process.exit(2);
  }
  const implFlag = get("--impl");
  if (implFlag !== undefined && implFlag !== "ts" && implFlag !== "pi-rs") {
    console.error(`run-tui: --impl 只接受 ts|pi-rs，收到 ${JSON.stringify(implFlag)}`);
    process.exit(2);
  }
  const implEnv = process.env.DIFTEST_IMPL;
  if (implFlag !== undefined && implEnv !== undefined && implFlag !== implEnv) {
    console.error(`run-tui: --impl=${implFlag} 与 DIFTEST_IMPL=${implEnv} 冲突，请只指定一个`);
    process.exit(2);
  }
  const impl = implFlag ?? implEnv ?? "ts";
  const scenarioPath = get("--scenario");
  const outDir = get("--out");
  if (!scenarioPath || !outDir) { console.error("用法: node run-tui.mjs --scenario <path> --out <dir>"); process.exit(2); }
  const scenario = JSON.parse(fs.readFileSync(scenarioPath, "utf8"));
  if (!scenario.tui) { console.error(`场景 ${scenario.id} 缺少 tui 字段`); process.exit(2); }
  const { cols, rows } = scenario.tui;
  const tmp = await fs.promises.mkdtemp(path.join(os.tmpdir(), `tui-${scenario.id}-`));
  for (const dir of ["home", "cfg", "sess", "work"]) await fs.promises.mkdir(path.join(tmp, dir), { recursive: true });
  for (const [rel, content] of Object.entries(scenario.files ?? {})) {
    const target = path.join(tmp, "work", rel);
    await fs.promises.mkdir(path.dirname(target), { recursive: true });
    await fs.promises.writeFile(target, content);
  }
  // P7-7d3：**预置会话文件**（与 `run.mjs:285-291` 同一形状：`{ fileName, content }`，
  // 内容里可写 `<CWD>` 占位符，由运行器替换成真实临时工作目录）。
  // 为什么需要：`/resume` 的列表渲染在本环境拿不到 TS golden —— harness 运行期本来没有会话文件
  // （会话在退出时才落盘，TS 在 All 作用域也只显示 `No sessions found`）。
  if (scenario.sessionFiles) {
    // 兼容两种形状：`{fileName, content}`（与 run.mjs 一致）与**数组**（一次预置多个会话，
    // 用于 `/resume` 的排序/列表等多会话场景）。
    const entries = Array.isArray(scenario.sessionFiles) ? scenario.sessionFiles : [scenario.sessionFiles];
    for (const entry of entries) {
      // `<SESS>` = 会话目录（绝对路径），用于构造 `parentSessionPath` 指向另一个预置会话。
      // `<NOW_MINUS:n>` = 现在减 n 秒的 ISO 时间戳。**为什么需要**：`/resume` 行尾显示
      // `3d`/`10mo` 这类相对时间，若夹具写死日期，golden 会随时间漂移；用相对 now 写死
      // 时间戳后，"写入时 now - n" 与"运行期 now - n" 只差启动那几秒 ⇒ 分档稳定
      // （例如 n=60 ⇒ 永远落 `1m`），且排序（按最后活动时间降序）确定。
      const seeded = entry.content
        .replaceAll("<CWD>", path.join(tmp, "work"))
        .replaceAll("<SESS>", path.join(tmp, "sess"))
        .replace(/<NOW_MINUS:(\d+)>/g, (_, seconds) =>
          new Date(Date.now() - Number(seconds) * 1000).toISOString(),
        );
      await fs.promises.writeFile(path.join(tmp, "sess", entry.fileName), seeded);
    }
  }
  // 注意：helper 的 cwd 在临时目录 ⇒ 输出/退出码路径必须是**绝对路径**（否则 ENOENT）。
  const runDir = path.join(path.resolve(outDir), "run-1");
  await fs.promises.mkdir(runDir, { recursive: true });
  const scriptPath = path.join(tmp, "keys.json");
  await fs.promises.writeFile(scriptPath, JSON.stringify(stepsToHelperScript(scenario.tui.steps), null, 2));
  const rawPath = path.join(runDir, "raw.bin");
  const codePath = path.join(runDir, "exit-code.txt");
  // 起 mock-provider（沿用 run.mjs 的思路）并写 cfg 的 models.json/settings.json 指向它。
  // requestLog 落到 --out 目录（不落 mkdtemp）⇒ 事后可取证（P7-3/W0(b) 教训）。
  const requestLog = path.join(path.resolve(outDir), "requests.jsonl");
  const marksPath = path.join(path.resolve(outDir), "marks.jsonl");
  const mockBin = process.env.MOCK_BIN ?? path.join(REPO, "target/debug/mock-provider");
  const mock = spawn(mockBin, [path.resolve(scenarioPath), "--port", "0", "--request-log", requestLog], { cwd: REPO, stdio: ["ignore", "pipe", "pipe"] });
  let mockStderr = "";
  mock.stderr.on("data", (chunk) => { mockStderr += chunk.toString(); process.stderr.write(`[mock] ${chunk}`); });
  const mockPort = await new Promise((resolve, reject) => {
    let buffer = "";
    const timer = setTimeout(() => reject(new Error("mock-provider 启动超时")), 15000);
    mock.stdout.on("data", (chunk) => {
      buffer += chunk.toString();
      // 必须取冒号后的端口：`listening 127.0.0.1:35539` 若用 /listening\s+(\d+)/ 会匹配到 IP 里的 127（P7-3 实测教训，表现为 TUI 报 Connection error）。
      const match = buffer.match(/listening\s+[\d.]+:(\d+)/);
      if (match) { clearTimeout(timer); resolve(Number.parseInt(match[1], 10)); }
    });
    mock.on("exit", (code) => { clearTimeout(timer); reject(new Error(`mock-provider 退出 code=${code}`)); });
  });
  const providerName = scenario.provider ?? "openai-completions";
  // P8e-10/F：场景可声明 `extraProviders`（`{ name: {api?, apiKey?, models?} }`），
  // 用于对拍"多 provider 时 footer 的 `(provider)` 前缀"——它们指向同一个 mock，
  // 只是 provider 名不同，因此 `getAvailableProviderCount() > 1`。
  const extraProviders =
    scenario.extraProviders && typeof scenario.extraProviders === "object"
      ? scenario.extraProviders
      : {};
  const providersJson = {
    [providerName]: {
      baseUrl: `http://127.0.0.1:${mockPort}/v1`,
      api: providerName,
      apiKey: "test-key",
      // `extraModels`：字符串或对象，**同 id 覆盖基础模型**（V3 起支持）。
      models: (() => {
        const extra = (scenario.extraModels ?? []).map((m) =>
          typeof m === "string" ? { id: m } : m,
        );
        const base = scenario.model ?? "mock-model";
        return [
          Object.assign({ id: base }, ...extra.filter((m) => m.id === base)),
          ...extra.filter((m) => m.id !== base),
        ];
      })(),
    },
  };
  for (const [name, spec] of Object.entries(extraProviders)) {
    providersJson[name] = {
      baseUrl: `http://127.0.0.1:${mockPort}/v1`,
      api: spec?.api ?? providerName,
      apiKey: spec?.apiKey ?? "test-key",
      models: Array.isArray(spec?.models) ? spec.models : [{ id: scenario.model ?? "mock-model" }],
    };
  }
  await fs.promises.writeFile(path.join(tmp, "cfg", "models.json"),
    JSON.stringify({ providers: providersJson }, null, 2));
  // 场景可以声明 `theme`（`dark`/`light`）：主题属于 settings.json 的配置，
  // 两侧必须喂**同一份**配置才能比较配色（ref `settings-manager.ts:782-791`）。
  const settingsJson = {
    defaultProvider: providerName,
    defaultModel: scenario.model ?? "mock-model",
  };
  if (typeof scenario.theme === "string" && scenario.theme.length > 0) {
    settingsJson.theme = scenario.theme;
  }
  // P8e-10/M-c：场景可以带 `settings` 对象，**逐键并入** settings.json
  // （例如 `enabledModels` 里放一个不存在的 id，用来对拍启动期 Warning）。
  // 只允许并入，不允许覆盖 runner 自己写的 provider/model（否则两侧不再同源）。
  if (scenario.settings && typeof scenario.settings === "object") {
    for (const [key, value] of Object.entries(scenario.settings)) {
      if (key === "defaultProvider" || key === "defaultModel") continue;
      settingsJson[key] = value;
    }
  }
  await fs.promises.writeFile(path.join(tmp, "cfg", "settings.json"),
    JSON.stringify(settingsJson, null, 2));

  const defaultCmd = impl === "pi-rs"
    ? path.join(REPO, "target/debug/pi-rs")
    : `node ${process.env.TS_PI_ENTRY ?? ""}`;
  // 场景可以用 `args` 声明额外命令行参数（例如 `T40-continue` 需要 `--continue`）。
  // 以前只能靠 `TUI_CMD` 环境变量带参数，导致"忘记带 `--continue` 就对不上 golden"的假差异。
  // 场景可以用既有的 `cliArgs` 声明命令行参数（print 路径一直是这个键）；
  // 以前 TUI 路径不读它，导致 `T40-continue` 少了 `--continue` 而出假差异。
  const scenarioArgs = Array.isArray(scenario.cliArgs) ? scenario.cliArgs.map(String) : [];
  //  覆盖可执行文件时，场景的  仍要**照常追加**（否则  场景
  // 用替代二进制跑会丢参数，表现为"历史不重放"的假差异）。
  const baseCmd = process.env.TUI_CMD ?? defaultCmd;
  const cmd = [baseCmd, ...scenarioArgs].join(" ");
  // **不在 run-tui 内部再套 unshare**：否则被测进程与 mock-provider 会落在**不同 netns**，
  // 127.0.0.1 互不可达（实测表现为 TS 报 `Error: Connection error.` 且 mock 收不到请求）。
  // 隔离由调用方统一提供：`unshare -n sh -c 'ip link set lo up; node run-tui.mjs …'`。
  const child = spawn("python3", [HELPER, "--cols", String(cols), "--rows", String(rows),
    "--out", rawPath, "--exit-code-file", codePath, "--script", scriptPath, "--marks", marksPath, "--", ...cmd.split(" ")], {
    cwd: path.join(tmp, "work"),
    // **颜色模式必须确定**：`COLORTERM` 决定真彩/256 色（ref `terminal-image.ts:72-75`），
    // 宿主机若有 `COLORTERM=truecolor`，两侧录制会落在不同色彩模式上（历史 golden 里
    // 真彩/256 各 29 个，正是这么来的）。这里统一清掉，保证两侧都走 256 色。
    // `DIFTEST_COLORTERM`：C1 需要按环境变量控制色彩模式（`unset` = 不设）。
    env: { ...process.env, COLORTERM: process.env.DIFTEST_COLORTERM === "unset" ? undefined : (process.env.DIFTEST_COLORTERM ?? ""), HOME: path.join(tmp, "home"),
      PI_CODING_AGENT_DIR: path.join(tmp, "cfg"), PI_CODING_AGENT_SESSION_DIR: path.join(tmp, "sess"),
      // 与 `run.mjs:342/586` 同源：docs 段的三个路径由 `PI_PACKAGE_DIR` 决定（它来自 `TS_PI_ENTRY`）。
      // 不传会让 pi-rs 的 docs 段退化（`/README.md` 等，差 213 字符），从而在会话层/请求层出现假差异。
      PI_PACKAGE_DIR: process.env.TS_PI_ENTRY
        ? path.dirname(path.dirname(path.dirname(process.env.TS_PI_ENTRY)))
        : "" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => { stderr += chunk.toString(); });
  const helperExit = await new Promise((resolve) => child.on("exit", resolve));
  // P7-4/W0e：读 helper 写的 mark 偏移（每行一条 JSON），落进 meta.json 的 snapshotOffsets。
  const snapshotOffsets = (await fs.promises.readFile(marksPath, "utf8").catch(() => ""))
    .split("\n").filter(Boolean).map((line) => JSON.parse(line));
  const exitCode = Number.parseInt(await fs.promises.readFile(codePath, "utf8").catch(() => "-1"), 10);
  // W4（会话层对拍）：会话目录在 mkdtemp 里，运行结束前把它复制到 --out 下，供事后比较 session.jsonl。
  const sessionCopy = path.join(path.resolve(outDir), "session");
  try {
    await fs.promises.rm(sessionCopy, { recursive: true, force: true });
    await fs.promises.cp(path.join(tmp, "sess"), sessionCopy, { recursive: true });
  } catch {
    // 没写会话（例如 --no-session）时忽略。
  }
  await fs.promises.writeFile(path.join(runDir, "meta.json"),
    JSON.stringify({ id: scenario.id, cols, rows, helperExit, exitCode, stderr, mockStderr, requestLog,
      snapshotOffsets, sessionDir: sessionCopy }, null, 2));
  mock.kill("SIGTERM");
  const rawBytes = fs.existsSync(rawPath) ? fs.statSync(rawPath).size : 0;
  console.log(`[run-tui] ${scenario.id} helperExit=${helperExit} measuredExit=${exitCode} rawBytes=${rawBytes}`);
  process.exit(helperExit === 0 ? 0 : 1);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
