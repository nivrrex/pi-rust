#!/usr/bin/env node
// scripts/check-scenarios.mjs —— 对 scenarios/*.json 做全量键名校验。
//
// 背景（D0）：mock 的 `Scenario` / `Step` 缺 `rename_all = "camelCase"`，
// serde 默认静默忽略未知字段——`gateAfterChunks` / `disconnectAfterChunks` /
// `retryAfter` 因此从未生效。这类错误不能靠人眼核对，必须机械拦截。
//
// 用法：node scripts/check-scenarios.mjs [--list-injected]

import fs from "node:fs";
import path from "node:path";

const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
// 默认校验仓库 `scenarios/`；`--dir <path>` 便于单测（P7-2 的 tui 字段校验）。
const dirArg = process.argv.indexOf("--dir");
const DIR = dirArg >= 0 ? path.resolve(process.argv[dirArg + 1]) : path.join(ROOT, "scenarios");

/** 场景顶层允许的键（与 `tools/mock-provider` 的 Scenario 结构体一致）。 */
const TOP_LEVEL = new Set([
  "id",
  "description",
  "provider",
  "model",
  "mode",
  "prompt",
  "reasoningModel",
  "cost",
  "contextWindow",
  "maxTokens",
  "settings",
  "commands",
  "followUp",
  "responses",
  "script",
  "files",
  "globalFiles",
  "sessionFiles",
  "extraModels",
  "cliArgs",
  "enableSkills",
  "enablePromptTemplates",
  "expectInjections",
  "tui",
  // 主题（`dark`/`light`）：写进被测进程的 settings.json，两侧同源比较配色。
  "theme",
  // P8e-10/F：额外的 provider（写进 models.json），用于对拍 footer 的 `(provider)` 前缀。
  "extraProviders",
]);

/** 每个响应条目允许的键（与 `Step` 结构体一致）。 */
const STEP = new Set([
  "file",
  "status",
  "retryAfter",
  "gateAfterChunks",
  "disconnectAfterChunks",
  "chunkByEvent",
]);

/** script 动作。 */
// `followUp` 的条目键名（X01/X02 新增 `impl`：分阶段换实现）。
const FOLLOW_UP_KEYS = new Set(["prompt", "cliArgs", "impl"]);

/** tui 段允许的键与动作（P7-2）。 */
const TUI_KEYS = new Set(["cols", "rows", "steps", "snapshots"]);
const TUI_ACTIONS = new Set(["waitScreen", "waitStrip", "waitStable", "type", "snapshot", "waitExit", "delay"]);
const TUI_STEP_KEYS = new Set(["action", "contains", "text", "name", "timeoutMs", "quietMs", "delayMs", "ms"]);

const SCRIPT_ACTIONS = new Set([
  "send",
  "createFile",
  "waitEvent",
  "waitState",
  "releaseGate",
  "waitExit",
]);

/** 曾经被静默忽略的键——列出使用它们的场景，决定哪些需要重录。 */
const PREVIOUSLY_IGNORED = ["gateAfterChunks", "disconnectAfterChunks", "retryAfter", "chunkByEvent"];

const problems = [];
const injected = [];

for (const name of fs.readdirSync(DIR).filter((f) => f.endsWith(".json")).sort()) {
  const file = path.join(DIR, name);
  let scenario;
  try {
    scenario = JSON.parse(fs.readFileSync(file, "utf-8"));
  } catch (error) {
    problems.push(`${name}: JSON 解析失败：${error.message}`);
    continue;
  }

  for (const key of Object.keys(scenario)) {
    if (!TOP_LEVEL.has(key)) problems.push(`${name}: 未知顶层键 ${key}`);
  }
  if (!Array.isArray(scenario.responses) || scenario.responses.length === 0) {
    problems.push(`${name}: responses 必须是非空数组`);
  } else {
    scenario.responses.forEach((step, index) => {
      for (const key of Object.keys(step)) {
        if (!STEP.has(key)) problems.push(`${name}: responses[${index}] 未知键 ${key}`);
      }
      if (PREVIOUSLY_IGNORED.some((k) => k in step)) {
        injected.push(`${name}: responses[${index}] 使用了 ${Object.keys(step).filter((k) => PREVIOUSLY_IGNORED.includes(k)).join(", ")}`);
      }
    });
  }
  (scenario.followUp ?? []).forEach((step, index) => {
    if (step === null || typeof step !== "object") {
      problems.push(`${name}: followUp[${index}] 必须是对象`);
      return;
    }
    for (const key of Object.keys(step)) {
      if (!FOLLOW_UP_KEYS.has(key)) problems.push(`${name}: followUp[${index}] 未知键 ${key}`);
    }
    if (step.impl !== undefined && !["same", "ts", "pi-rs", "other"].includes(step.impl)) {
      problems.push(`${name}: followUp[${index}].impl 只接受 ts / pi-rs / other`);
    }
  });

  for (const step of scenario.script ?? []) {
    if (!SCRIPT_ACTIONS.has(step.action)) {
      problems.push(`${name}: script 动作 ${step.action} 未知`);
    }
  }

  // P7-2：TUI 场景字段（`tui{cols,rows,steps,snapshots}`）。未知键/缺字段/非法尺寸一律报错，
  // 避免拼错的键被静默忽略（与 `deny_unknown_fields` 同精神）。
  if (scenario.tui !== undefined) {
    const tui = scenario.tui;
    if (tui === null || typeof tui !== "object" || Array.isArray(tui)) {
      problems.push(`${name}: tui 必须是对象`);
    } else {
      for (const key of Object.keys(tui)) {
        if (!TUI_KEYS.has(key)) problems.push(`${name}: tui 未知键 ${key}`);
      }
      for (const key of ["cols", "rows"]) {
        const value = tui[key];
        if (!Number.isInteger(value) || value < 1 || value > 1000) {
          problems.push(`${name}: tui.${key} 必须是 1..1000 的整数（实际 ${JSON.stringify(value)}）`);
        }
      }
      if (!Array.isArray(tui.steps) || tui.steps.length === 0) {
        problems.push(`${name}: tui.steps 必须是非空数组`);
      } else {
        tui.steps.forEach((step, index) => {
          if (step === null || typeof step !== "object") {
            problems.push(`${name}: tui.steps[${index}] 必须是对象`);
            return;
          }
          if (!TUI_ACTIONS.has(step.action)) {
            problems.push(`${name}: tui.steps[${index}] 动作 ${step.action} 未知`);
          }
          for (const key of Object.keys(step)) {
            if (!TUI_STEP_KEYS.has(key)) problems.push(`${name}: tui.steps[${index}] 未知键 ${key}`);
          }
        });
      }
      const snapshotNames = new Set((tui.steps ?? [])
        .filter((step) => step !== null && typeof step === "object" && step.action === "snapshot")
        .map((step) => step.name));
      if (!Array.isArray(tui.snapshots) || tui.snapshots.length === 0) {
        problems.push(`${name}: tui.snapshots 必须是非空数组`);
      } else {
        const declared = new Set((tui.steps ?? [])
          .filter((step) => step !== null && typeof step === "object" && step.action === "snapshot")
          .map((step) => step.name));
        for (const snapshot of tui.snapshots) {
          if (!declared.has(snapshot)) {
            problems.push(`${name}: tui.snapshots 里的 ${snapshot} 没有对应的 snapshot 步骤`);
          }
        }
        void snapshotNames;
      }
    }
  }
}

// `followUp` 的条目也要查键名：X01/X02 新增了 `impl`（分阶段换实现），
// 拼错或写成别的名字会被静默忽略——那正是 D0（`gateAfterChunks` 从未生效）的教训。

if (process.argv.includes("--list-injected")) {
  console.log("使用了曾被静默忽略的键的场景：");
  for (const line of injected) console.log(`  ${line}`);
  if (injected.length === 0) console.log("  （无）");
}

if (problems.length > 0) {
  console.error("场景键名校验失败：");
  for (const problem of problems) console.error(`  ${problem}`);
  process.exit(1);
}
console.log(`check-scenarios: ${fs.readdirSync(DIR).filter((f) => f.endsWith(".json")).length} 个场景键名全部合法`);
