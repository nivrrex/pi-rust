// tui-oracle：`/resume` 会话选择器的**真实 ref 类**渲染夹具（P9-1-A / I2）。
//
// 目的：给"会话数 > maxVisible"的分页行为（窗口截取、`(n/total)` 提示、PageUp/PageDown）
// 提供**由 ref 真实 `SessionSelectorComponent` 产出**的期望行，禁止手写夹具。
//
// 用法：
//   node --import ./tools/difftest/tui-oracle/register.mjs \
//        tools/difftest/tui-oracle/gen-session-selector.mjs \
//        tools/difftest/tui-oracle/fixtures-session-selector.json
//
// 输出：JSON（含 SGR 原文与剥离 SGR 的纯文本两种形式）+ 文本侧车（Rust 单测直接读 txt）。
import fs from "node:fs";
import path from "node:path";

const REF = process.env.PI_TUI_REF_DIR
	? path.resolve(process.env.PI_TUI_REF_DIR, "..", "..")
	: path.resolve(path.dirname(new URL(import.meta.url).pathname), "../../../ref/pi-0.87.1/packages/coding-agent/src");
const componentPath = path.join(REF, "modes/interactive/components/session-selector.ts");
const keybindingsPath = path.join(REF, "core/keybindings.ts");

const { SessionSelectorComponent } = await import(componentPath);
const { KeybindingsManager } = await import(keybindingsPath);
// ref 的组件依赖全局主题单例：夹具只用 `dark`（文本内容与主题无关，样式另行剥离 SGR）。
const { initTheme } = await import(path.join(REF, "modes/interactive/theme/theme.ts"));
initTheme("dark");

const SGR = /\u001b\[[0-9;]*m/g;
const strip = (line) => line.replace(SGR, "");

const WIDTH = 100;
const COUNT = 12; // > maxVisible（ref 默认 10）
// `modified` 取"生成时刻 - 400 天"：两侧都渲染成 `1y`，夹具在一年内稳定
// （超过 730 天会变 `2y`，届时重新生成即可）。
const LONG_AGO = Date.now() - 400 * 86_400_000;

function sessions() {
  return Array.from({ length: COUNT }, (_, i) => ({
    path: `/sessions/s${String(i).padStart(2, "0")}.jsonl`,
    id: `session-${i}`,
    cwd: "/work/project",
    name: undefined,
    parentSessionPath: undefined,
    created: new Date(LONG_AGO),
    modified: new Date(LONG_AGO),
    messageCount: 1 + i,
    firstMessage: `会话 ${i + 1}：分页夹具`,
    allMessagesText: `会话 ${i + 1}`,
  }));
}

async function make() {
  const kb = new KeybindingsManager({});
  const rows = sessions();
  // 两个 loader 都是"立刻返回固定 12 条"：夹具不碰真实文件系统。
  const loader = async (onProgress) => {
    onProgress?.(rows.length, rows.length, rows);
    return rows;
  };
  const component = new SessionSelectorComponent(
    loader,
    loader,
    () => {},
    () => {},
    () => {},
    () => {},
    { keybindings: kb, showRenameHint: false },
    undefined,
  );
  // 构造器立刻发起 `loadScope("current")`（异步）⇒ 等它落地再渲染。
  for (let i = 0; i < 50; i++) {
    const text = component.render(WIDTH).join("\n");
    if (text.includes("会话 1：分页夹具")) break;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  return component;
}

// 按键字节：ref 的 `tui.select.down` 默认键是 down 箭头；pageDown 是 pageDown 键。
const DOWN = "\u001b[B";
const PAGE_DOWN = "\u001b[6~";
const PAGE_UP = "\u001b[5~";

// 按键用**名字**记录（Rust 侧逐条重放同一序列，不靠猜）。
const KEY_BYTES = { down: DOWN, pagedown: PAGE_DOWN, pageup: PAGE_UP };
const CASES = [
  { id: "initial", keys: [] },
  { id: "down-3", keys: Array(3).fill("down") },
  { id: "down-8", keys: Array(8).fill("down") },
  { id: "pagedown-1", keys: ["pagedown"] },
  { id: "pagedown-2", keys: ["pagedown", "pagedown"] },
  { id: "pagedown-then-pageup", keys: ["pagedown", "pagedown", "pageup"] },
  { id: "down-to-last", keys: Array(COUNT + 3).fill("down") },
];

const cases = [];
for (const { id, keys } of CASES) {
  const component = await make();
  for (const key of keys) component.handleInput(KEY_BYTES[key]);
  const raw = component.render(WIDTH);
  cases.push({ id, keys, raw, lines: raw.map(strip) });
}

const payload = {
  source: "ref/pi-0.87.1 packages/coding-agent/src/modes/interactive/components/session-selector.ts",
  width: WIDTH,
  count: COUNT,
  cases,
};

const out = process.argv[2] ?? "fixtures-session-selector.json";
fs.writeFileSync(out, JSON.stringify(payload, null, 2));

// 文本侧车（Rust 无 serde 依赖也能读）
const sidecar = [];
sidecar.push(`# width=${WIDTH} count=${COUNT}`);
for (const c of cases) {
  sidecar.push(`case ${c.id} keys=${c.keys.join(",")}`);
    for (const line of c.lines) sidecar.push(`line ${Buffer.from(line, "utf8").toString("base64")}`);
}
fs.writeFileSync(out.replace(/\.json$/, ".lines.txt"), sidecar.join("\n") + "\n");
console.log(`✓ ${out}（${cases.length} 个用例，宽度 ${WIDTH}，会话 ${COUNT} 个）`);
