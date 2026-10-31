// P7-6c/W2：**内容口径**诊断（只读；不改既有比较器）。
//
// 目的：对"末屏行数不一致"的场景，判断两侧**内容**是否只是**整屏位移**：
//   取 golden 帧的有效行序列，判断它是否为 pi-rs 同帧行序列（两侧都经既有掩码/归一化）的**连续切片**，
//   命中则输出 offset；否则报"内容不一致"；两侧长度关系不成立（golden 比 pi-rs 长）时报"无法判定"。
// 每帧必须被判定（脚本内部断言，不允许空过）。
//
// 用法：node tools/difftest/compare-tui-content.mjs --left <golden 目录> --right <pi-rs 目录> [--cols 100] [--rows 30] [--json]
import fs from "fs";
import { renderScreen, normalizeScreen, snapOffsetToFrameEnd, trimRawToOffset } from "./compare-tui.mjs";

function load(dir, rows, cols) {
  const base = fs.existsSync(`${dir}/run-1/run-1/raw.bin`) ? `${dir}/run-1/run-1` : dir;
  // pi-rs/golden 的"行序列"用**较大 rows** 渲染，尽量拿到录制里的全部内容（不裁剪）。
  const raw = fs.readFileSync(`${base}/raw.bin`);
  const meta = JSON.parse(fs.readFileSync(`${base}/meta.json`, "utf8"));
  return (meta.snapshotOffsets ?? []).map((item) => {
    const snapped = snapOffsetToFrameEnd(raw, item.offset);
    return normalizeScreen(renderScreen(trimRawToOffset(raw, snapped ?? raw.length), cols, rows));
  });
}

function effective(lines) {
  return lines.filter((line) => line.trim() !== "");
}

function sliceOffset(haystack, needle) {
  if (needle.length === 0) return 0;
  for (let start = 0; start + needle.length <= haystack.length; start += 1) {
    let match = true;
    for (let k = 0; k < needle.length; k += 1) {
      if (haystack[start + k] !== needle[k]) { match = false; break; }
    }
    if (match) return start;
  }
  return -1;
}

/**
 * 判定一帧（P7-6e/W1 收紧后的规则）：
 * **"纯位移"必须同时满足**：
 *   ① pi-rs 的有效行序列是 golden 的**连续切片**；
 *   ② 被缺失的行**全部位于 golden 的顶部（前缀）**，即 `offset + pi-rs 行数 == golden 行数`
 *      （切片两端都不能有 golden 的残余行）。
 * 缺行落在中部或末尾 ⇒ 一律判"内容不一致"。
 * （旧版曾把"golden 是 pi-rs 的超集/子集"两个方向都当纯位移，见 notes/progress.md 的旧输出对照。）
 */
export function judge(goldenFrame, pirsFrame) {
  const g = effective(goldenFrame);
  const r = effective(pirsFrame);
  const offset = sliceOffset(g, r);
  if (offset < 0) {
    return { verdict: "内容不一致", reason: "pi-rs 行序列不是 golden 的连续切片", goldenLines: g.length, pirsLines: r.length };
  }
  const tail = g.length - (offset + r.length);
  if (tail !== 0) {
    return {
      verdict: "内容不一致",
      reason: `pi-rs 是 golden 的切片，但 golden 在切片之后仍有 ${tail} 行（缺行不在顶部）`,
      offset,
      goldenLines: g.length,
      pirsLines: r.length,
    };
  }
  return { verdict: "内容一致且纯位移", offset, goldenLines: g.length, pirsLines: r.length };
}

function main() {
  const argv = process.argv.slice(2);
  const get = (name, fallback) => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : fallback; };
  const left = get("--left"); const right = get("--right");
  if (!left || !right) { console.error("用法: node tools/difftest/compare-tui-content.mjs --left <golden> --right <pi-rs>"); process.exit(2); }
  const cols = Number.parseInt(get("--cols", "100"), 10);
  const rows = Number.parseInt(get("--rows", "30"), 10); // P7-6d：两侧必须同 rows（raw 在不同 rows 下重放结果不同）
  const gframes = load(left, rows, cols); const rframes = load(right, rows, cols);
  const results = [];
  for (let i = 0; i < Math.max(gframes.length, rframes.length); i += 1) {
    if (!gframes[i] || !rframes[i]) { results.push({ frame: i, verdict: "无法判定", reason: "两侧帧数不同" }); continue; }
    results.push({ frame: i, ...judge(gframes[i], rframes[i]) });
  }
  // 断言：每个帧都必须被判定（不允许空过）。
  for (let i = 0; i < Math.max(gframes.length, rframes.length); i += 1) {
    if (!results[i] || !results[i].verdict) throw new Error(`帧 ${i} 未被判定`);
  }
  if (argv.includes("--json")) console.log(JSON.stringify(results, null, 2));
  else for (const item of results) {
    const detail = item.offset !== undefined ? `offset=${item.offset}` : item.reason ?? "";
    console.log(`帧${item.frame}: ${item.verdict} ${detail}（golden ${item.goldenLines ?? "?"} 行 / pi-rs ${item.pirsLines ?? "?"} 行）`);
  }
  process.exit(0);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
