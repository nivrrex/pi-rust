// P7-5d/W2：**逐帧**比较（只读复用 compare-tui 的导出；不改它的默认行为）。
//
// 为什么需要：`compare-tui.mjs` 只比**最后一个快照帧**（P7-4h/W1 的截断口径），
// 于是"中间帧写错、末帧又抵消回来"的缺陷（例如 T11 的 Ctrl+W 范围）看不见；
// 本工具按 `meta.json.snapshotOffsets` 逐帧渲染、逐帧归一化，并对**每一帧**做同样的有效产出闸门。
//
// 用法：node tools/difftest/compare-tui-frames.mjs --left <目录> --right <目录> [--cols 100] [--rows 30]
//       [--require <条目>]... [--json]
// 退出码：0=所有帧 same；1=存在不同帧或闸门失败；2=用法/输入错误。
import fs from "fs";
import {
  renderScreen,
  normalizeScreen,
  snapOffsetToFrameEnd,
  trimRawToOffset,
  gate,
} from "./compare-tui.mjs";

function loadFrame(dir, index, cols, rows) {
  const dirPath = fs.existsSync(`${dir}/run-1/run-1/raw.bin`) ? `${dir}/run-1/run-1` : dir;
  const raw = fs.readFileSync(`${dirPath}/raw.bin`);
  const meta = JSON.parse(fs.readFileSync(`${dirPath}/meta.json`, "utf8"));
  const offsets = meta.snapshotOffsets ?? [];
  if (index >= offsets.length) return null;
  const snapped = snapOffsetToFrameEnd(raw, offsets[index].offset);
  if (snapped === null) throw new Error(`偏移之前没有 ?2026l 帧边界：${dirPath} 帧${index}`);
  return normalizeScreen(renderScreen(trimRawToOffset(raw, snapped), cols, rows));
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
    console.error("用法: node tools/difftest/compare-tui-frames.mjs --left <目录> --right <目录> [--cols 100] [--rows 30] [--require X]...");
    process.exit(2);
  }
  const cols = Number.parseInt(get("--cols", "100"), 10);
  const rows = Number.parseInt(get("--rows", "30"), 10);
  const required = argv.reduce((acc, item, index) => {
    if (item === "--require") acc.push(argv[index + 1]);
    return acc;
  }, []);

  const results = [];
  for (let index = 0; ; index += 1) {
    const a = loadFrame(left, index, cols, rows);
    const b = loadFrame(right, index, cols, rows);
    if (a === null && b === null) break;
    if (a === null || b === null) {
      results.push({ index, status: "invalid", reason: `两侧帧数不同（左 ${a === null ? "缺" : "有"} / 右 ${b === null ? "缺" : "有"}）` });
      continue;
    }
    // 每一帧都过同样的闸门：目标条目必须出现在该帧里。
    const gateResult = gate(a, b, required);
    if (!gateResult.ok) {
      results.push({ index, status: "invalid", reason: gateResult.reason ?? "闸门失败" });
      continue;
    }
    let status = "same";
    let firstDiff = null;
    for (let y = 0; y < Math.max(a.length, b.length); y += 1) {
      if ((a[y] ?? "") !== (b[y] ?? "")) {
        status = "different";
        firstDiff = { row: y, left: (a[y] ?? "").slice(0, 60), right: (b[y] ?? "").slice(0, 60) };
        break;
      }
    }
    results.push({ index, status, firstDiff });
  }

  const same = results.filter((item) => item.status === "same").length;
  if (argv.includes("--json")) {
    console.log(JSON.stringify({ frames: results.length, same, results }, null, 2));
  } else {
    console.log("帧序号 判定 首差异（左 / 右）");
    for (const item of results) {
      const detail = item.firstDiff
        ? `行${item.firstDiff.row}: L=${JSON.stringify(item.firstDiff.left)} R=${JSON.stringify(item.firstDiff.right)}`
        : item.reason ?? "";
      console.log(`${String(item.index).padStart(4)} ${item.status.padEnd(9)} ${detail}`);
    }
    console.log(`[compare-tui-frames] 帧数=${results.length} same=${same} ${same === results.length ? "全部一致" : "存在差异"}`);
  }
  process.exit(same === results.length ? 0 : 1);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
