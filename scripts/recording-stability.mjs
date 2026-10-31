#!/usr/bin/env node
// scripts/recording-stability.mjs —— 基线"录制稳定性"报告脚本（P4-14 步骤 1 落库版）。
//
// 用途：对同一批 `golden/_runs/<id>/run-{1,2,3}` 用**同一份脚本**按不同判据各算一遍，
// 把口径差异摊开（P4-11 起生效的主口径 = `normalized`）。
//
// 判据（`--criterion`）：
//   raw             逐行**原始文本**相同（不做任何归一化）
//   meta            读 `golden/<id>/meta.json` 的 `sane` + `stableLayers`（录制期判据）
//   normalized      经**比较器**归一化后 A/B/C/fs 四层逐层相同（= 主口径）
//   normalized-text 在 `normalized` 之上再忽略 `message_start.message.content` 里的空 text 块
//                  （**仅用于评估** S25-anthropic 的候选规则，未落地到比较器）
//
// 用法：node scripts/recording-stability.mjs [--criterion raw|meta|normalized|normalized-text] [--all]
//   `--all` 同时打印每个场景的判定与理由；默认只打印计数与"不稳"名单。
import fs from "node:fs";
import path from "node:path";
import { compareScenario, STATUS } from "../tools/difftest/compare.mjs";

const args = process.argv.slice(2);
const criterion = args.includes("--criterion") ? args[args.indexOf("--criterion") + 1] : "normalized";
const showAll = args.includes("--all");
const runRoot = "golden/_runs";

/** 与 runner 对齐：`ZZ-*` 是自测夹具；`T-cost`（P4-13 的临时探针）与 `S09`（P4-15 已退役，
 * 见 `notes/p415-step5-residuals.md` 与 `notes/decisions.md`）都只剩录制目录、没有 `scenarios/*.json`，
 * 因此不计入"场景"。（目录本身保留，不删除。） */
function recordingIds() {
  return fs
    .readdirSync(runRoot)
    .filter((id) => fs.existsSync(path.join(runRoot, id, "run-1")) && !id.startsWith("ZZ"))
    .sort();
}

/** 场景清单（`scenarios/*.json`），用来把"录制目录"折成"场景"。 */
function scenarioIds() {
  return fs
    .readdirSync("scenarios")
    .filter((name) => name.endsWith(".json"))
    .map((name) => name.slice(0, -".json".length))
    .sort();
}

function readLines(file) {
  return fs
    .readFileSync(file, "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "");
}

/** 逐行归一化：解析不出 JSON 的行（runner 写的分段标记）原样保留。 */
function normalizeLines(lines, { dropUsage, dropEmptyTextBlock }) {
  return lines.map((line) => {
    let value;
    try {
      value = JSON.parse(line);
    } catch {
      return line.trim();
    }
    if (dropUsage) {
      const strip = (node) => {
        if (Array.isArray(node)) return node.map(strip);
        if (node && typeof node === "object") {
          return Object.fromEntries(
            Object.entries(node)
              .filter(([key]) => key !== "usage")
              .map(([key, inner]) => [key, strip(inner)]),
          );
        }
        return node;
      };
      value = strip(value);
    }
    if (
      dropEmptyTextBlock &&
      value?.type === "message_start" &&
      Array.isArray(value.message?.content)
    ) {
      value.message.content = value.message.content.filter(
        (block) => !(block?.type === "text" && block.text === ""),
      );
    }
    return JSON.stringify(value, Object.keys(value).sort());
  });
}

const layerFiles = { A: "session.jsonl", B: "events.jsonl", C: "requests.jsonl", fs: "fs.json" };

/** 判据 `raw` / `normalized(-text)` 的实现：逐层比对 run-2、run-3 与 run-1。 */
async function layerVerdict(id) {
  const base = path.join(runRoot, id);
  const bad = new Set();
  for (const layer of Object.keys(layerFiles)) {
    const file = layerFiles[layer];
    const first = path.join(base, "run-1", file);
    if (!fs.existsSync(first)) continue;
    for (const n of [2, 3]) {
      const other = path.join(base, `run-${n}`, file);
      if (!fs.existsSync(other)) {
        bad.add(`${layer}:缺录制`);
        continue;
      }
      if (criterion === "raw") {
        if (fs.readFileSync(first, "utf8") !== fs.readFileSync(other, "utf8")) bad.add(layer);
        continue;
      }
      const options = { dropUsage: true, dropEmptyTextBlock: criterion === "normalized-text" };
      const a = normalizeLines(readLines(first), options);
      const b = normalizeLines(readLines(other), options);
      if (a.length !== b.length || a.some((line, index) => line !== b[index])) bad.add(layer);
    }
  }
  return [...bad];
}

/** 判据 `normalized` 用比较器本体（口径与验收完全一致），避免"自己写一套归一化"。 */
async function comparatorVerdict(id) {
  const base = path.join(runRoot, id);
  const bad = new Set();
  for (const n of [2, 3]) {
    const dir = path.join(base, `run-${n}`);
    if (!fs.existsSync(path.join(dir, "events.jsonl"))) {
      bad.add("缺录制");
      continue;
    }
    const result = await compareScenario(path.join(base, "run-1"), dir);
    for (const layer of ["A", "B", "C", "fs"]) {
      if (result[layer].status !== STATUS.same) bad.add(layer);
    }
  }
  return [...bad];
}

function metaVerdict(id) {
  const file = path.join("golden", id, "meta.json");
  if (!fs.existsSync(file)) return ["无 meta.json"];
  const meta = JSON.parse(fs.readFileSync(file, "utf8"));
  const bad = [];
  if (!meta.sane) bad.push("sane=false");
  for (const [layer, ok] of Object.entries(meta.stableLayers ?? {})) {
    if (!ok) bad.push(layer);
  }
  return bad;
}

const ids = recordingIds();
const scenarios = scenarioIds();
const rows = [];
for (const id of ids) {
  let bad;
  if (criterion === "meta") bad = metaVerdict(id);
  else if (criterion === "normalized") bad = await comparatorVerdict(id);
  else bad = await layerVerdict(id);
  rows.push({ id, bad, isScenario: scenarios.includes(id) });
}

const stable = rows.filter((row) => row.bad.length === 0);
const unstable = rows.filter((row) => row.bad.length > 0);
const stableScenarios = stable.filter((row) => row.isScenario);
const scenarioRows = rows.filter((row) => row.isScenario);

console.log(`判据: ${criterion}`);
console.log(
  `录制目录 ${rows.length} 个（其中场景 ${scenarioRows.length} 个；非场景目录：${rows
    .filter((row) => !row.isScenario)
    .map((row) => row.id)
    .join(",") || "无"}）`,
);
console.log(`稳定: ${stable.length}/${rows.length}（仅场景：${stableScenarios.length}/${scenarioRows.length}）`);
if (showAll) {
  for (const row of rows) {
    console.log(`  ${row.bad.length === 0 ? "稳定" : "不稳"} ${row.id}${row.bad.length ? " → " + row.bad.join(",") : ""}`);
  }
} else {
  for (const row of unstable) console.log(`  不稳: ${row.id} → ${row.bad.join(",")}`);
}
