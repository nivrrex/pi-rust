// W4（P7-4d）：TUI 的**会话层**与**请求层**对拍。
//
// 用途：`compare-tui.mjs` 比的是"渲染后的屏幕"；本工具比的是两侧落盘/发出的原始材料：
//   ① 会话层：`<dir>/session/*.jsonl` 的**条目种类序列**与逐条规范化 JSON；
//   ② 请求层：`<dir>/requests.jsonl`（mock-provider 记录的请求体）。
// 判据：逐层 same/different；任一侧缺文件即报错退出（**不静默**——缺失不能被当成相同）。
// 归一化只读复用 `normalize.mjs`（`canonicalizeJsonLines`），不改任何既有文件。
import fs from "fs";
import path from "path";
import { canonicalizeJsonLines } from "./normalize.mjs";

function readSession(dir) {
  const sessionDir = path.join(dir, "session");
  if (!fs.existsSync(sessionDir)) throw new Error(`缺会话目录：${sessionDir}`);
  const files = fs.readdirSync(sessionDir).filter((name) => name.endsWith(".jsonl"));
  if (files.length === 0) throw new Error(`会话目录里没有 .jsonl：${sessionDir}`);
  const lines = [];
  for (const name of files) {
    lines.push(...fs.readFileSync(path.join(sessionDir, name), "utf8").split("\n").filter(Boolean));
  }
  return lines;
}

function readRequests(dir) {
  const file = path.join(dir, "requests.jsonl");
  if (!fs.existsSync(file)) throw new Error(`缺请求日志：${file}`);
  return fs.readFileSync(file, "utf8").split("\n").filter(Boolean);
}

/**
 * 易变字段掩码（只作用于本工具的**比较输入**，不改 `normalize.mjs`、不影响屏幕比较器）：
 * 临时目录、ISO 时间戳、UUID（会话 id）、短十六进制 id、会话文件路径。
 * 与 `notes/decisions.md` 里 TUI 掩码规则同源（`<TMP>/<TIMESTAMP>/<UUID>/<ID-FRAGMENT>`）。
 */
function maskVolatile(text) {
  return text
    .replace(/\/tmp\/tui-[A-Za-z0-9-]+/g, "<TMP>")
    .replace(/[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z/g, "<TIMESTAMP>")
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, "<UUID>")
    .replace(/"timestamp":[0-9]{6,}/g, '"timestamp":"<TS>"')
    .replace(/"[0-9a-f]{8}"/g, '"<ID-FRAGMENT>"')
    .replace(/"<ID-FRAGMENT>"/g, '"<ID-FRAGMENT>"');
}

/** 把 JSONL 行解析成对象数组；解析失败即报错（不吞）。 */
function parseLines(lines, label) {
  return lines.map((line, index) => {
    try {
      return JSON.parse(line);
    } catch (error) {
      throw new Error(`${label} 第 ${index + 1} 行不是合法 JSON：${error.message}`);
    }
  });
}

/** 条目种类序列（会话层的"骨架"）：`type` + `message.role` + `message.sections` 的键。 */
function kindSequence(entries) {
  return entries.map((entry) => {
    const message = entry.message ?? {};
    const sections = message.sections && typeof message.sections === "object"
      ? Object.keys(message.sections).sort().join(",")
      : "";
    return [entry.type ?? "?", message.role ?? ""].join(":") + (sections ? `[${sections}]` : "");
  });
}

function compareList(left, right, label) {
  const diffs = [];
  const max = Math.max(left.length, right.length);
  for (let index = 0; index < max; index += 1) {
    const a = left[index] ?? "<缺>";
    const b = right[index] ?? "<缺>";
    if (a !== b) diffs.push({ index, left: a, right: b });
  }
  return { label, same: diffs.length === 0, count: [left.length, right.length], diffs };
}

export function compareTuiLayers(leftDir, rightDir) {
  const session = maskVolatile(canonicalizeJsonLines(readSession(leftDir).join("\n"))).trim().split("\n").filter(Boolean);
  const sessionOther = maskVolatile(canonicalizeJsonLines(readSession(rightDir).join("\n"))).trim().split("\n").filter(Boolean);
  const kinds = compareList(
    kindSequence(parseLines(readSession(leftDir), "会话")),
    kindSequence(parseLines(readSession(rightDir), "会话")),
    "会话条目种类",
  );
  const entries = compareList(session, sessionOther, "会话条目（规范化 JSON）");
  const requests = compareList(
    maskVolatile(canonicalizeJsonLines(readRequests(leftDir).join("\n"))).trim().split("\n").filter(Boolean),
    maskVolatile(canonicalizeJsonLines(readRequests(rightDir).join("\n"))).trim().split("\n").filter(Boolean),
    "请求体（规范化 JSON）",
  );
  return { kinds, entries, requests, same: kinds.same && entries.same && requests.same };
}

function main() {
  const argv = process.argv.slice(2);
  const get = (name, fallback) => {
    const index = argv.indexOf(name);
    return index >= 0 ? argv[index + 1] : fallback;
  };
  const left = get("--left");
  const right = get("--right");
  if (!left || !right) {
    console.error("用法: node tools/difftest/compare-tui-layers.mjs --left <ts目录> --right <pi-rs目录> [--json]");
    process.exit(2);
  }
  const result = compareTuiLayers(left, right);
  if (argv.includes("--json")) {
    console.log(JSON.stringify(result, null, 2));
  } else {
    for (const layer of [result.kinds, result.entries, result.requests]) {
      console.log(`[layer] ${layer.label}: ${layer.same ? "same" : "different"}（行数 ${layer.count[0]} vs ${layer.count[1]}）`);
      for (const diff of layer.diffs.slice(0, 3)) {
        console.log(`  行${diff.index}: L=${JSON.stringify(String(diff.left).slice(0, 110))}`);
        console.log(`         R=${JSON.stringify(String(diff.right).slice(0, 110))}`);
      }
    }
  }
  console.log(`[compare-tui-layers] ${result.same ? "same" : "different"}`);
  process.exit(result.same ? 0 : 1);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
