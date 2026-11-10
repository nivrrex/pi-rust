// F52 场景生成器：合成会话（思考块 + 含换行的超长 bash + 60 行输出 + edit），
// 覆盖 ctrl+o（工具行展开/回折）与 ctrl+t（思考块隐藏/显示）两个按键的两种配置
// （合成扩展 / `-ne`）与两种宽度（110 / 80），另加 light 一份。
//
// 只生成场景 JSON；golden 一律由 TS 侧录制，不手写。
import { readFileSync, writeFileSync } from "node:fs";

const collapseExt = readFileSync("tools/difftest/fixtures/f52-collapse-ext.ts", "utf8");
const extFiles = {
  "f52-collapse-ext/index.ts": collapseExt,
  "f52-collapse-ext/package.json":
    '{\n "name": "f52-collapse-ext",\n "version": "0.0.0",\n "type": "module",\n "exports": "./index.ts"\n}',
};

const prelude = (text) => [
  { action: "waitScreen", contains: "Pi can explain", timeoutMs: 30000 },
  { action: "waitStable", quietMs: 1200, timeoutMs: 20000 },
  { action: "type", text: `${text}\r` },
  { action: "delay", ms: 7000 },
  { action: "waitStable", quietMs: 1500, timeoutMs: 20000 },
];
const press = (key, name) => [
  { action: "type", text: key },
  { action: "delay", ms: 1200 },
  { action: "waitStable", quietMs: 900, timeoutMs: 15000 },
  { action: "delay", ms: 800 },
  { action: "waitStable", quietMs: 1200, timeoutMs: 15000 },
];
const tail = (name) => [
  { action: "snapshot", name },
  { action: "type", text: "\u0004" },
  { action: "waitStable", quietMs: 600, timeoutMs: 15000 },
];
const CTRL_O = "\u000f";
const CTRL_T = "\u0014";

const base = ({ id, description, cols, rows, withExtension, keys, settings, theme }) => {
  const scenario = {
    id,
    description,
    provider: "openai-completions",
    model: "mock-model",
    mode: "tui",
    prompt: "",
    files: { "AGENTS.md": "# 项目规则\n", "synth.txt": "旧\n" },
    globalFiles: {},
    cliArgs: withExtension ? [] : ["-ne"],
    responses: [
      { file: "fixtures/sse/openai/f52-bash-heredoc.sse", chunkByEvent: true },
      { file: "fixtures/sse/openai/text-basic.sse" },
    ],
    tui: { cols, rows, snapshots: ["state"], steps: [...prelude("hi"), ...keys.flatMap((k) => press(k, "state")), ...tail("state")] },
  };
  if (withExtension) scenario.extensions = extFiles;
  if (settings) scenario.settings = settings;
  if (theme) scenario.theme = theme;
  return scenario;
};

// `rows` 取 170：展开态含 60 行 bash 输出 + 完整启动帮助，40 行的窗口会滚动，
// 而两侧的**物理滚动行数**不由渲染内容决定（`Working` 行残留），会造成与渲染无关的
// 一行动位差（实测 40 行时固定差 1 行，170 行时逐行一致）。
const ROWS = 170;
const cases = [
  ["F52a-ctrl-o-once", "F52：ctrl+o 一次，工具行切到原生块（含换行命令与 60 行输出）", 110, true, [CTRL_O], { hideThinkingBlock: true }],
  ["F52b-ctrl-o-twice", "F52：ctrl+o 两次，回到折叠行（来回不丢）", 110, true, [CTRL_O, CTRL_O], { hideThinkingBlock: true }],
  ["F52c-ctrl-t-once", "F52：ctrl+t 一次，思考块从隐藏切到全文", 110, true, [CTRL_T], { hideThinkingBlock: true }],
  ["F52d-ctrl-t-twice", "F52：ctrl+t 两次，思考块回到隐藏态", 110, true, [CTRL_T, CTRL_T], { hideThinkingBlock: true }],
  ["F52e-alternate", "F52：ctrl+o 与 ctrl+t 交替，两种折叠互不干扰", 110, true, [CTRL_O, CTRL_T, CTRL_O], { hideThinkingBlock: true }],
  ["F52f-ne-ctrl-o-once", "F52：-ne（无扩展）下 ctrl+o 展开", 110, false, [CTRL_O], { hideThinkingBlock: true }],
  ["F52g-ctrl-o-w80", "F52：80 列下 ctrl+o 展开（长命令折行）", 80, true, [CTRL_O], { hideThinkingBlock: true }],
];

for (const [id, description, cols, withExtension, keys, settings] of cases) {
  const scenario = base({ id, description, cols, rows: ROWS, withExtension, keys, settings });
  writeFileSync(`scenarios-tui/${id}.json`, `${JSON.stringify(scenario, null, 1)}\n`);
}

// light：同一形态换主题，放 `scenarios-tui-light/`（regress 的 light 套件按目录取场景）。
const light = base({
  id: "L-F52a-ctrl-o-once",
  description: "F52：light 主题下 ctrl+o 展开",
  cols: 110,
  rows: ROWS,
  withExtension: true,
  keys: [CTRL_O],
  settings: { hideThinkingBlock: true },
  theme: "light",
});
writeFileSync("scenarios-tui-light/L-F52a-ctrl-o-once.json", `${JSON.stringify(light, null, 1)}\n`);
console.log("generated 8 scenarios");
