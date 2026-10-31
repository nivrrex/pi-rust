//! 场景清单对账：`scenarios/` 里的**每个**场景，在结果表里必须恰好占一行；缺一即失败。
//!
//! 由来：P3 的"全部一致"被推翻过两次 —— 一次是 B 层从未比对，一次是**原始场景从未跑**
//! （表里的 S13b 是派生场景 S13b-text/S13b-toolcall，原始 S13b/S13c 没跑）。
//! 人工核对不可靠，这里做成机械对账：清单**从目录列**，不凭记忆。
//!
//! 用法：
//!   node tools/difftest/inventory.mjs --out <pi-rs 产出目录>      # 结果行 = 该目录下的场景子目录
//!   node tools/difftest/inventory.mjs --rows S01,S02,...          # 手工给的已比较清单
//! 退出码 0 = 一一对应；1 = 有缺或多；2 = 用法错误。

import { readdir, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "..", "..");

/** `scenarios/` 下的场景 id 列表（去掉 `.json`），按名排序。 */
export async function listScenarios(scenarioDir = path.join(repoRoot, "scenarios")) {
  const names = await readdir(scenarioDir);
  return names
    .filter((name) => name.endsWith(".json"))
    .map((name) => name.slice(0, -".json".length))
    .sort();
}

/** 从产出目录里收集"已比较"的场景 id（每个场景一个子目录；`_` 开头的是运行记录，不算）。 */
export async function collectResultRows(outRoot) {
  const names = await readdir(outRoot);
  const rows = [];
  for (const name of names) {
    if (name.startsWith("_")) continue;
    const full = path.join(outRoot, name);
    const info = await stat(full).catch(() => null);
    if (info?.isDirectory()) rows.push(name);
  }
  return rows.sort();
}

/**
 * 对账：返回 `{ ok, missing, extra, compared }`。
 * `missing` = 在 `scenarios/` 里但结果表没有；`extra` = 结果表里有但 `scenarios/` 没有。
 */
export function reconcile(scenarios, rows) {
  const scenarioSet = new Set(scenarios);
  const rowSet = new Set(rows);
  const missing = scenarios.filter((name) => !rowSet.has(name));
  const extra = rows.filter((name) => !scenarioSet.has(name));
  return { ok: missing.length === 0 && extra.length === 0, missing, extra, compared: rows.length };
}

/** 对账结果的可读报告（一行一句，缺一即列出名字）。 */
export function formatReconciliation(result, scenarios) {
  const lines = [];
  lines.push(`场景清单对账：scenarios/ 共 ${scenarios.length} 个，结果表 ${result.compared} 行`);
  if (result.missing.length > 0) {
    lines.push(`  缺少（scenarios/ 里有、结果表没有，${result.missing.length} 个）：${result.missing.join(" ")}`);
  }
  if (result.extra.length > 0) {
    lines.push(`  多余（结果表里有、scenarios/ 没有，${result.extra.length} 个）：${result.extra.join(" ")}`);
  }
  lines.push(result.ok ? "  一一对应" : "  **对账失败**");
  return lines.join("\n");
}

async function main() {
  const argv = process.argv.slice(2);
  let outRoot = null;
  let rowsArg = null;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--out") outRoot = argv[++i];
    else if (arg === "--rows") rowsArg = argv[++i];
    else {
      console.error("用法: node tools/difftest/inventory.mjs --out <目录> | --rows a,b,c");
      process.exit(2);
    }
  }
  if (!outRoot && !rowsArg) {
    console.error("用法: node tools/difftest/inventory.mjs --out <目录> | --rows a,b,c");
    process.exit(2);
  }
  const scenarios = await listScenarios();
  const rows = outRoot
    ? await collectResultRows(outRoot)
    : rowsArg.split(",").map((name) => name.trim()).filter(Boolean);
  const result = reconcile(scenarios, rows);
  console.log(formatReconciliation(result, scenarios));
  process.exit(result.ok ? 0 : 1);
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))) {
  await main();
}
