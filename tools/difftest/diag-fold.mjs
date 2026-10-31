#!/usr/bin/env node
// tools/difftest/diag-fold.mjs —— **只读诊断**：把 B 层的"折叠后行号"对齐回原始事件行号。
//
// 目的：`compare.mjs` 的 B 层会先归一化（`canonicalizeJsonLines`）再折叠
// （`foldStreamingIncrements`），所以它报的"第 N 行"**不是**原始事件行号。本脚本只读地
// 复现同一条流水线，并保留"折叠行 → 原始行号"的映射，用于定位差异到底落在哪一条事件上。
//
// **不改判据**：只 import 现有函数；本地折叠只是为了保留行号映射，并与 imported 的
// `foldStreamingIncrements` 做一致性自检（不一致就直接报错，不静默）。
//
// 用法：node tools/difftest/diag-fold.mjs <场景> --out <产出目录> [--limit N]

import fs from "node:fs";
import path from "node:path";
import {
  canonicalizeJsonLines,
  foldStreamingIncrements,
  stripStreamingUsage,
} from "./normalize.mjs";

const args = process.argv.slice(2);
const scenario = args[0];
const outIndex = args.indexOf("--out");
const outDir = outIndex >= 0 ? args[outIndex + 1] : "tmp";
const limitIndex = args.indexOf("--limit");
const limit = limitIndex >= 0 ? Number(args[limitIndex + 1]) : 12;
if (!scenario) {
  console.error("用法：node tools/difftest/diag-fold.mjs <场景> --out <产出目录> [--limit N]");
  process.exit(2);
}

const goldenFile = path.join("golden", scenario, "events.jsonl");
const actualFile = path.join(outDir, scenario, "events.jsonl");

/** 非空行。 */
const nonEmpty = (text) => text.split("\n").filter((line) => line.trim() !== "");

/**
 * 与 `foldStreamingIncrements` **同规则**，但保留"折叠行 → 原始行号"的映射。
 * 规则见 `normalize.mjs:43-65`：可折叠的是 `message_update`/`tool_execution_update`，
 * 同一类型**连续**出现时只留第一条。
 */
function foldWithMap(preparedText, rawText) {
  const prepared = nonEmpty(preparedText);
  const raw = nonEmpty(rawText);
  const out = [];
  for (let index = 0; index < prepared.length; index += 1) {
    const line = prepared[index];
    let type = null;
    try {
      type = JSON.parse(line).type;
    } catch {
      type = null;
    }
    const foldable = type === "message_update" || type === "tool_execution_update";
    const previous = out[out.length - 1];
    if (foldable && previous && previous.type === type) {
      previous.count += 1;
      previous.rawLines.push(index + 1);
      continue;
    }
    out.push({
      type,
      line,
      count: foldable ? 1 : 0,
      rawLines: [index + 1],
      rawLine: raw[index] ?? "",
    });
  }
  return out;
}

/** 逐字段差异（解析后；两侧解析失败时退回整行比较）。 */
function fieldDiff(leftLine, rightLine) {
  let left;
  let right;
  try {
    left = JSON.parse(leftLine);
    right = JSON.parse(rightLine);
  } catch {
    return "（一侧不是 JSON，只能整行比较）";
  }
  const walk = (a, b, at) => {
    if (a === b) return null;
    if (typeof a !== typeof b) return `${at}: 类型不同`;
    if (a && b && typeof a === "object") {
      const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
      for (const key of keys) {
        if (!(key in a)) return `${at}.${key}: golden 缺、pi-rs 有`;
        if (!(key in b)) return `${at}.${key}: pi-rs 缺、golden 有`;
        const deeper = walk(a[key], b[key], `${at}.${key}`);
        if (deeper) return deeper;
      }
      return null;
    }
    const sa = JSON.stringify(a);
    const sb = JSON.stringify(b);
    const cut = [...sa].findIndex((ch, i) => ch !== [...sb][i]);
    return `${at}: 值不同\n      golden=…${sa.slice(Math.max(0, cut - 50), cut + 110)}\n      pi-rs =…${sb.slice(Math.max(0, cut - 50), cut + 110)}`;
  };
  return walk(left, right, "") ?? "（解析后相同 ⇒ 差异在键序或空白）";
}

function prepare(text) {
  // 与 compare.mjs 的 B 层同一顺序：canonicalize → stripStreamingUsage → fold
  const canonical = canonicalizeJsonLines(text);
  const stripped = stripStreamingUsage(canonical);
  const folded = foldWithMap(stripped, text);
  const imported = foldStreamingIncrements(stripped);
  const mine = folded.map((entry) => entry.line).join("\n");
  if (mine !== imported) {
    throw new Error("本地折叠与 imported foldStreamingIncrements 不一致（诊断脚本自身有误）");
  }
  return folded;
}

const goldenText = fs.readFileSync(goldenFile, "utf8");
const actualText = fs.readFileSync(actualFile, "utf8");
const golden = prepare(goldenText);
const actual = prepare(actualText);

console.log(`场景 ${scenario}：golden 折叠后 ${golden.length} 行、pi-rs ${actual.length} 行`);
const show = (entry) =>
  `raw[${entry.rawLines.join(",")}] ${entry.type ?? "?"} :: ${entry.line.slice(0, 150)}`;
console.log(`—— golden 前 ${limit} 行 ——`);
golden.slice(0, limit).forEach((entry, index) => console.log(`${index + 1}. ${show(entry)}`));
console.log(`—— pi-rs 前 ${limit} 行 ——`);
actual.slice(0, limit).forEach((entry, index) => console.log(`${index + 1}. ${show(entry)}`));

const count = Math.max(golden.length, actual.length);
for (let index = 0; index < count; index += 1) {
  const left = golden[index];
  const right = actual[index];
  if (left?.line === right?.line) continue;
  console.log(`\n=== 首个不同：折叠后第 ${index + 1} 行 ===`);
  console.log(`golden raw[${left ? left.rawLines.join(",") : "-"}]`);
  console.log(`  未归一化原文：${(left?.rawLine ?? "<缺>").slice(0, 400)}`);
  console.log(`  归一化后原文：${(left?.line ?? "<缺>").slice(0, 400)}`);
  console.log(`pi-rs raw[${right ? right.rawLines.join(",") : "-"}]`);
  console.log(`  未归一化原文：${(right?.rawLine ?? "<缺>").slice(0, 400)}`);
  console.log(`  归一化后原文：${(right?.line ?? "<缺>").slice(0, 400)}`);
  if (left && right) {
    console.log(`逐字段差异：${fieldDiff(left.line, right.line)}`);
  }
  break;
}
