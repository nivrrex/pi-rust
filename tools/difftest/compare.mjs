//! golden 与 pi-rs 产出的逐层比对。
//!
//! 用法：`node tools/difftest/compare.mjs <场景id> [--out <pi-rs 产出目录>] [--json]`
//!
//! **它为什么必须存在**：`run.mjs` 只负责**录制**，仓库里原本没有任何东西把 pi-rs 的产出
//! 与 golden 比过。没有比对器时，"某场景通过"只能靠人肉 diff，而人肉 diff 不会去做
//! 归一化、不会折叠 B 层增量、也不会在**缺文件**时报失败——这三点恰好都是差分测试里
//! 最容易自欺的地方（见 notes/decisions.md 里那次误把 `stableLayers` 当成 golden 比对的记录）。
//!
//! 分层（PLAN 第 3 节）：
//! - **A**：`session.jsonl` 的语义比较**留到 P4**（本次不比较 A 层内容，只比 fs 与退出码）；
//! - **B**：`events.jsonl`，按 PLAN 第 3 节折叠**连续的**流式增量事件后再比；
//! - **C**：`requests.jsonl`（发给 provider 的请求体）；
//! - **fs**：文件系统副作用快照 `fs.json`；
//! - **退出码**：`meta.json` 的 `exitCode`。
//!
//! `cost`/`usage` 等数值**不参与任何归一化**（PLAN 第 3 节要求逐位比较）。

import { readFile } from "node:fs/promises";
import { existsSync, statSync } from "node:fs";
import path from "node:path";

import {
  canonicalizeJsonLines,
  foldStreamingIncrements,
  stripEmptyMessageStartTextBlocks,
  stripStreamingUsage,
} from "./normalize.mjs";

/** 逐层比较的结果。[status] 取值：`same` / `different` / `missing` / `not_compared`。 */
export const STATUS = {
  same: "same",
  different: "different",
  missing: "missing",
  notCompared: "not_compared",
};

function firstDifference(left, right) {
  const a = left.split("\n");
  const b = right.split("\n");
  const length = Math.max(a.length, b.length);
  for (let i = 0; i < length; i += 1) {
    if (a[i] !== b[i]) {
      return {
        line: i + 1,
        golden: a[i] === undefined ? "<缺行>" : a[i].slice(0, 200),
        actual: b[i] === undefined ? "<缺行>" : b[i].slice(0, 200),
      };
    }
  }
  return null;
}

async function readOrNull(file) {
  try {
    return await readFile(file, "utf8");
  } catch {
    return null;
  }
}

/**
 * 比较一个文件的文本层。
 *
 * **缺文件一律算失败**（`missing`），不得当成"相同"——这是本次自测里专门钉住的一条：
 * 少一个产出文件时，"两边都是空"最容易被误判成一致。
 */
function compareText(
  goldenText,
  actualText,
  { fold = false, dropEmptyTextBlocks = false } = {},
) {
  if (goldenText === null || actualText === null) {
    return { status: STATUS.missing, detail: "一侧缺文件" };
  }
  // 先做键序归一化（Q8：键顺序不是判据），再按需要折叠流式增量。
  const canonicalLeft = canonicalizeJsonLines(goldenText);
  const canonicalRight = canonicalizeJsonLines(actualText);
  // B 层：先按 Q18 的规则抹掉流式事件的 usage，再折叠连续增量。
  const preparedLeft = fold ? stripStreamingUsage(canonicalLeft) : canonicalLeft;
  const preparedRight = fold ? stripStreamingUsage(canonicalRight) : canonicalRight;
  // Q-S25 规则（P4-15 步骤 3）：只对 anthropic 场景、只在 `message_start` 的空串 text 块上启用。
  const prunedLeft = dropEmptyTextBlocks
    ? stripEmptyMessageStartTextBlocks(preparedLeft)
    : preparedLeft;
  const prunedRight = dropEmptyTextBlocks
    ? stripEmptyMessageStartTextBlocks(preparedRight)
    : preparedRight;
  const left = fold ? foldStreamingIncrements(prunedLeft) : prunedLeft;
  const right = fold ? foldStreamingIncrements(prunedRight) : prunedRight;
  if (left === right) return { status: STATUS.same };
  return { status: STATUS.different, detail: firstDifference(left, right) };
}

/**
 * 比较一个场景的全部层。
 *
 * `goldenDir` 与 `actualDir` 都是"场景目录"（直接含 session.jsonl/events.jsonl/requests.jsonl/fs.json/meta.json）。
 */
/**
 * A 层：`session.jsonl` 的**逐行语义比较**。
 *
 * 口径（RUNBOOK-5 第 2 节，不得放宽）：
 * - **行数不同即失败**——不允许"跳过缺失行"；
 * - 比较前做键序归一化（Q8：键顺序不是判据）；`id`/`parentId`/`targetId`/`timestamp`
 *   由 runner 在写 golden 时替换成占位符，这里只做键序归一化；
 * - **缺文件算失败**（`missing`），不得当成"相同"。
 *
 * 负向对照见 `test/compare.test.mjs`：少一行、多一行、`parentId` 改错、
 * `replacement` 改值，四种都必须 `different`。
 */
export function compareSessionLines(goldenText, actualText) {
  if (goldenText === null || actualText === null) {
    return { status: STATUS.missing, detail: "一侧缺 session.jsonl" };
  }
  // **A 层只归一化键序，但 `parameters` 例外**：TS 的 `declarationsEqual`
  // （`ref/.../ai/src/utils/transcript.ts:140-141`）用 `JSON.stringify` 比对工具声明，
  // 键序**参与**判等——归一化掉它就等于把 X02 那类缺陷放过去。
  const canonicalize = (text) =>
    canonicalizeJsonLines(text, { preserveObjectKeys: new Set(["parameters"]) })
      .split("\n")
      .filter((line) => line.trim() !== "");
  const left = canonicalize(goldenText);
  const right = canonicalize(actualText);
  if (left.length !== right.length) {
    return {
      status: STATUS.different,
      detail: `行数不同：golden ${left.length} 行、pi-rs ${right.length} 行`,
    };
  }
  for (let i = 0; i < left.length; i += 1) {
    if (left[i] !== right[i]) {
      return {
        status: STATUS.different,
        detail: `第 ${i + 1} 行不同`,
        line: i + 1,
        golden: left[i].slice(0, 200),
        actual: right[i].slice(0, 200),
      };
    }
  }
  return { status: STATUS.same, detail: `${left.length} 行逐行一致` };
}

/**
 * 有效产出闸门（Q22）：任一侧"没有真的跑起来"时返回原因字符串，否则 null。
 *
 * 为什么必须有它：两侧都失败（产出目录里只有 FAILED.md、各 jsonl 全 0 字节）时，
 * 逐层比较会退化成"空 vs 空"并判 `same`——把**运行失败**伪装成**全同**。
 * 这正是本项目反复强调的"测试通过不等于被测路径真的走到了"。
 */
async function invalidSideReason(goldenDir, actualDir) {
  for (const [label, dir] of [["golden", goldenDir], ["pi-rs", actualDir]]) {
    if (!existsSync(dir)) return `${label} 侧产出目录不存在：${dir}`;
    if (existsSync(path.join(dir, "FAILED.md"))) {
      return `${label} 侧**运行失败**（目录里有 FAILED.md）`;
    }
    for (const name of ["session.jsonl", "events.jsonl", "requests.jsonl"]) {
      const file = path.join(dir, name);
      // 文件**不存在**仍交给各层原有的 missing 语义（不改既有行为）；
      // 只有"文件在但为空"才是本闸门要拦的"没跑起来"。
      if (!existsSync(file)) return null;
      const stat = statSync(file);
      if (stat.size === 0) return `${label} 侧的 ${name} 是 0 字节（没跑起来）`;
    }
  }
  return null;
}

/** 把闸门的原因铺到所有层上：一律 different，且写明原因。 */
function gateLayers(reason) {
  const layer = { status: STATUS.different, detail: `产出无效：${reason}` };
  return { A: { ...layer }, B: { ...layer }, C: { ...layer }, fs: { ...layer }, exit: { ...layer } };
}

export async function compareScenario(goldenDir, actualDir) {
  const invalid = await invalidSideReason(goldenDir, actualDir);
  if (invalid) return gateLayers(invalid);
  const layers = {};
  // B 层的 Q-S25 规则只在 anthropic 场景启用（判据来自 `meta.json` 的 `provider`）。
  const goldenMetaForProvider = await readOrNull(path.join(goldenDir, "meta.json"));
  const actualMetaForProvider = await readOrNull(path.join(actualDir, "meta.json"));
  const providerOf = (text) => {
    try {
      return JSON.parse(text ?? "{}").provider ?? null;
    } catch {
      return null;
    }
  };
  // 逐次录制目录（`golden/_runs/<id>/run-N`）里**没有** `meta.json`，所以还要一个不依赖它的判据：
  // 事件载荷里的 `api` 字段（助手消息自带 `"api":"anthropic-messages"`）。两者都拿不到时不启用规则。
  const goldenEventsText = await readOrNull(path.join(goldenDir, "events.jsonl"));
  const actualEventsText = await readOrNull(path.join(actualDir, "events.jsonl"));
  const apiFromEvents = (text) => {
    if (text === null) return null;
    if (text.includes('"api":"anthropic-messages"')) return "anthropic-messages";
    if (text.includes('"api":"openai-completions"')) return "openai-completions";
    return null;
  };
  const provider =
    providerOf(goldenMetaForProvider) ??
    providerOf(actualMetaForProvider) ??
    apiFromEvents(goldenEventsText) ??
    apiFromEvents(actualEventsText);
  const dropEmptyTextBlocks = provider === "anthropic-messages";

  // A 层：session.jsonl 逐行语义比较（P4 起纳入验收；此前是 not_compared）。
  layers.A = compareSessionLines(
    await readOrNull(path.join(goldenDir, "session.jsonl")),
    await readOrNull(path.join(actualDir, "session.jsonl")),
  );

  layers.B = compareText(goldenEventsText, actualEventsText, {
    fold: true,
    dropEmptyTextBlocks,
  });

  layers.C = compareText(
    await readOrNull(path.join(goldenDir, "requests.jsonl")),
    await readOrNull(path.join(actualDir, "requests.jsonl")),
  );

  layers.fs = compareText(
    await readOrNull(path.join(goldenDir, "fs.json")),
    await readOrNull(path.join(actualDir, "fs.json")),
  );

  // 退出码取自 meta.json；只比这一个字段，不比 meta 的其余内容（runs/stepTrace 等与实现无关）。
  const goldenMeta = await readOrNull(path.join(goldenDir, "meta.json"));
  const actualMeta = await readOrNull(path.join(actualDir, "meta.json"));
  if (goldenMeta === null || actualMeta === null) {
    layers.exit = { status: STATUS.missing, detail: "一侧缺 meta.json" };
  } else {
    const read = (text) => {
      try {
        return JSON.parse(text).exitCode;
      } catch {
        return "<解析失败>";
      }
    };
    const g = read(goldenMeta);
    const a = read(actualMeta);
    layers.exit =
      g === a
        ? { status: STATUS.same }
        : { status: STATUS.different, detail: { golden: g, actual: a } };
  }

  return layers;
}

/** 汇总：某一场景是否全部应比的层都一致。 */
export function allComparableLayersMatch(layers) {
  return Object.entries(layers)
    .filter(([, value]) => value.status !== STATUS.notCompared)
    .every(([, value]) => value.status === STATUS.same);
}

/**
 * 拒绝**绝对路径**或**仓库外**的产出目录。
 *
 * 起因（真实事故）：`--out /tmp/x` 传给 `run.mjs` 时，`path.join(REPO, outRoot, id)`
 * **不会**因绝对路径重置，产出被写进仓库里的 `tmp/x/` 下。这里显式拦掉，
 * 免得比对器读到的是"并不存在"的目录却毫无提示。
 */
export function assertSafeOutDir(outRoot, repoRoot) {
  if (path.isAbsolute(outRoot)) {
    throw new Error(`--out 不接受绝对路径（会经 path.join 落进仓库）：${outRoot}`);
  }
  const resolved = path.resolve(repoRoot, outRoot);
  const prefix = repoRoot.endsWith(path.sep) ? repoRoot : `${repoRoot}${path.sep}`;
  if (!resolved.startsWith(prefix)) {
    throw new Error(`--out 必须位于仓库内：${outRoot}`);
  }
  return resolved;
}

const main = async () => {
  const args = process.argv.slice(2);
  const id = args[0];
  if (!id) {
    console.error("用法: node tools/difftest/compare.mjs <场景id> [--out <目录>] [--json]");
    process.exit(2);
  }
  let outRoot = "tmp/pi-rs-out";
  let asJson = false;
  for (let i = 1; i < args.length; i += 1) {
    if (args[i] === "--out") outRoot = args[i + 1];
    if (args[i] === "--json") asJson = true;
  }
  const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..", "..");
  const actualRoot = assertSafeOutDir(outRoot, repoRoot);
  const layers = await compareScenario(
    path.join(repoRoot, "golden", id),
    path.join(actualRoot, id),
  );
  if (asJson) {
    console.log(JSON.stringify({ id, layers }, null, 2));
  } else {
    const summary = Object.entries(layers)
      .map(([name, value]) => `${name}=${value.status}`)
      .join(" ");
    console.log(`[compare] ${id}: ${summary}`);
    for (const [name, value] of Object.entries(layers)) {
      if (value.status === STATUS.different) {
        console.log(`  ${name} 首个差异（第 ${value.detail.line} 行）：`);
        console.log(`    golden: ${value.detail.golden}`);
        console.log(`    pi-rs : ${value.detail.actual}`);
      }
      if (value.status === STATUS.missing) {
        console.log(`  ${name}: ${value.detail}`);
      }
    }
  }
  process.exitCode = allComparableLayersMatch(layers) ? 0 : 1;
};

if (import.meta.url === `file://${process.argv[1]}`) {
  await main();
}
