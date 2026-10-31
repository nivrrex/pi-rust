// P7-2 W2 单测：tui 场景字段校验、compare-tui 的归一化与有效产出闸门、PTY 退出码回传。
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { compareTui, normalizeScreen, renderScreen, lastSnapshotOffset, trimRawToOffset, snapOffsetToFrameEnd } from "../compare-tui.mjs";

const REPO = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../../..");

function scenarioDir(payloads) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sc-check-"));
  for (const [name, payload] of Object.entries(payloads)) {
    fs.writeFileSync(path.join(dir, `${name}.json`), JSON.stringify(payload));
  }
  return dir;
}

const base = JSON.parse(fs.readFileSync(path.join(REPO, "scenarios/S52.json"), "utf8"));
const okTui = {
  cols: 100, rows: 30,
  steps: [{ action: "waitScreen", contains: "> " }, { action: "snapshot", name: "reply" }],
  snapshots: ["reply"],
};

test("tui 字段：合法场景通过", () => {
  const dir = scenarioDir({ ok: { ...base, id: "ok", tui: okTui } });
  const run = spawnSync("node", [path.join(REPO, "scripts/check-scenarios.mjs"), "--dir", dir], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stdout + run.stderr);
  assert.match(run.stdout, /1 个场景键名全部合法/);
});

test("tui 字段：未知键 / 非法尺寸 / 未知动作 / 快照不匹配 / 缺 steps 全部被拒", () => {
  const cases = {
    "unknown-key": { ...okTui, cols2: 1 },
    "bad-cols": { ...okTui, cols: 0 },
    "bad-action": { ...okTui, steps: [{ action: "nope" }] },
    "bad-snap": { ...okTui, snapshots: ["y"] },
    "missing-steps": { cols: 100, rows: 30, snapshots: ["reply"] },
  };
  for (const [name, tui] of Object.entries(cases)) {
    const dir = scenarioDir({ [name]: { ...base, id: name, tui } });
    const run = spawnSync("node", [path.join(REPO, "scripts/check-scenarios.mjs"), "--dir", dir], { encoding: "utf8" });
    assert.equal(run.status, 1, `${name} 必须被拒：${run.stdout}`);
    assert.match(run.stderr, new RegExp(name), `${name} 的错误信息应含场景名`);
  }
});

test("回放器：底行换行按真实终端滚屏（K1\'/H-A 性质测试）", async () => {
  const { renderStyledScreen } = await import("../compare-style.mjs");
  const rows = 4;
  const cols = 6;
  // 写满 rows+2 行：真实终端会把前两行滚出视口，最后两行可见。
  const stream = "A\r\nB\r\nC\r\nD\r\nE\r\nF\r\n";
  const styled = renderStyledScreen(stream, cols, rows)
    .map((row) => row.map((cell) => (typeof cell === "string" ? cell : cell.ch)).join("").trimEnd());
  assert.deepEqual(styled, ["D", "E", "F", ""]);
  // 两个回放器对同一输入必须给出同一屏（文本层）。
  const { renderScreen } = await import("../compare-tui.mjs");
  const plain = renderScreen(Buffer.from(stream, "utf8"), cols, rows).map((line) => line.trimEnd());
  assert.deepEqual(styled, plain);
});

test("回放器：终态与帧切分无关（整屏一次写 vs 逐行写）", async () => {
  const { renderStyledScreen } = await import("../compare-style.mjs");
  const rows = 4;
  const cols = 8;
  const whole = renderStyledScreen("one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n", cols, rows)
    .map((row) => row.map((cell) => (typeof cell === "string" ? cell : cell.ch)).join("").trimEnd());
  const piecewise = renderStyledScreen("one\r\n", cols, rows)
    .map((row) => row.map((cell) => (typeof cell === "string" ? cell : cell.ch)).join("").trimEnd());
  assert.notDeepEqual(whole, piecewise);
  const whole2 = renderStyledScreen("one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n", cols, rows)
    .map((row) => row.map((cell) => (typeof cell === "string" ? cell : cell.ch)).join("").trimEnd());
  assert.deepEqual(whole, whole2);
});

test("compare-tui：归一化（尺寸/颜色/spinner/时长/行尾/空行）", () => {
  const cols = 100;
  const rows = 30;
  // `renderScreen` 保留行尾空格（裁剪是 `normalizeScreen` 的职责）⇒ 这里用归一化后的结果断言。
  const rendered = normalizeScreen(renderScreen(Buffer.from("\x1b[2Jhi"), cols, rows));
  assert.equal(rendered[0], "hi");
  const left = normalizeScreen(["  a  ", "", "", "b", ""]);
  assert.deepEqual(left, ["  a", "", "b"]);
  const a = Buffer.from("\x1b[2J> \u280b working 1.2s\nreply ok\n");
  const b = Buffer.from("\x1b[2J> \u283f working 9.9s\nreply ok\n");
  assert.equal(compareTui(a, b, { cols, rows, required: ["> "] }).status, "same");
});

test("compare-tui：有效产出闸门（两侧空判 invalid，缺目标判 invalid）", () => {
  const empty = compareTui(Buffer.from(""), Buffer.from(""), { cols: 100, rows: 30, required: [] });
  assert.equal(empty.status, "invalid", "两侧空必须 invalid，绝不 same");
  const oneEmpty = compareTui(Buffer.from("\x1b[2Jhi"), Buffer.from(""), { cols: 100, rows: 30, required: [] });
  assert.equal(oneEmpty.status, "invalid", "一侧空必须 invalid");
  const missing = compareTui(Buffer.from("\x1b[2Jhi"), Buffer.from("\x1b[2Jhi"), { cols: 100, rows: 30, required: ["NOPE"] });
  assert.equal(missing.status, "invalid");
  assert.match(missing.reason, /缺目标条目/);
});

test("PTY helper：真实退出码回传（含非零码）", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-test-"));
  for (const code of [0, 7]) {
    const out = path.join(dir, `raw-${code}.bin`);
    const codeFile = path.join(dir, `code-${code}.txt`);
    const script = path.join(dir, `keys-${code}.json`);
    fs.writeFileSync(script, JSON.stringify([]));
    const run = spawnSync("unshare", ["-n", "python3", path.join(REPO, "tools/difftest/pty-helper.py"),
      "--cols", "80", "--rows", "24", "--out", out, "--exit-code-file", codeFile, "--script", script,
      "--", "python3", "-c", `import sys; sys.stdout.write("hi\\n"); sys.exit(${code})`], { encoding: "utf8" });
    assert.equal(run.status, 0, `helper 自身应成功：${run.stderr}`);
    assert.equal(fs.readFileSync(codeFile, "utf8").trim(), String(code), "退出码必须原样回传");
  }
});

// ---- P7-4/W0e：按 snapshot 偏移截断（含真实 raw 的负向对照） ----
test("trimRawToOffset 确实改变渲染：真实 T05 raw 截到 50% 时含 footer/cwd，全量渲染不含", () => {
  const raw = fs.readFileSync(path.join(REPO, "golden-tui/T05/run-1/raw.bin"));
  const full = normalizeScreen(renderScreen(raw, 100, 30)).join("\n");
  const half = normalizeScreen(renderScreen(trimRawToOffset(raw, Math.floor(raw.length * 0.5)), 100, 30)).join("\n");
  // 负向对照的**方向**必须可断言：截断后能看到中途画过的 footer 与 cwd（说明截断真的改了屏幕内容），
  // 而不截断的"最终屏幕"里它们已被后画的内容覆盖（这是 T05 golden 缺两行的真实机制）。
  assert.ok(half.includes("0.0%/128k"), "截到 50% 应能看到 footer 用量段");
  assert.ok(half.includes("work"), "截到 50% 应能看到 cwd");
  assert.ok(!full.includes("0.0%/128k"), "全量渲染里 footer 应已被覆盖");
  assert.notEqual(full, half, "截断与不截断的渲染必须不同");
});

test("trimRawToOffset 的边界（负向对照）", () => {
  assert.equal(trimRawToOffset(Buffer.from("abcd"), null).toString(), "abcd");
  assert.equal(trimRawToOffset(Buffer.from("abcd"), 999).toString(), "abcd");
  assert.equal(trimRawToOffset(Buffer.from("abcd"), 2).toString(), "ab");
  assert.equal(lastSnapshotOffset("/nonexistent/meta.json"), null);
});

// ---- P7-4c：偏移回退到最后一个同步帧结束（含负向对照） ----
test("snapOffsetToFrameEnd：偏移落在第二帧中间时回退到第一帧尾", () => {
  const f1 = "\u001b[?2026h AAAA \u001b[?2026l";
  const f2 = "\u001b[?2026h BBBB \u001b[?2026l";
  const raw = Buffer.from(f1 + f2 + "tail");
  const midFrame2 = Buffer.byteLength(f1) + 12; // 第二帧**帧体内已写出部分内容**处（+8 是帧头长度，再取 4 字节内容）
  const snapped = snapOffsetToFrameEnd(raw, midFrame2);
  assert.equal(snapped, Buffer.byteLength(f1), "应回退到第一帧的 ?2026l 结尾");
  const afterSnap = normalizeScreen(renderScreen(trimRawToOffset(raw, snapped), 100, 30)).join("\n");
  const withoutSnap = normalizeScreen(renderScreen(trimRawToOffset(raw, midFrame2), 100, 30)).join("\n");
  const firstFrameOnly = normalizeScreen(renderScreen(Buffer.from(f1), 100, 30)).join("\n");
  assert.equal(afterSnap, firstFrameOnly, "回退后应等于只渲染第一帧");
  assert.notEqual(withoutSnap, afterSnap, "负向对照：不回退则与回退结果不同");
});

test("snapOffsetToFrameEnd 的 invalid 分支（偏移之前无 ?2026l）", () => {
  const raw = Buffer.from("没有任何同步帧的普通输出");
  assert.equal(snapOffsetToFrameEnd(raw, raw.length), null, "无 ?2026l 必须判 invalid（null），不得退回全量");
  assert.equal(snapOffsetToFrameEnd(Buffer.from("abc"), 999), null, "越界偏移亦为 null");
});

// ---- P7-4g/W1：底行换行必须整屏上滚（含负向对照） ----
test("renderScreen：底行换行时整屏上滚", () => {
  const raw = Buffer.from("AAA\r\nBBB\r\nCCC\r\nDDD");
  const lines = renderScreen(raw, 5, 3).map((l) => l.trimEnd());
  // 三行屏幕依次写入 A/B/C 后在底行再换行 ⇒ A 应被滚出，屏幕为 B/C/D
  assert.deepEqual(lines, ["BBB", "CCC", "DDD"], "底行换行必须上滚（不回退则会是 ['AAA','BBB','DDD'] 或丢行）");
  // 负向对照：不滚动（旧的 clamp 行为）会得到下面这个结果，与上滚不同
  const noScroll = ["AAA", "BBB", "DDD"];
  assert.notDeepEqual(lines, noScroll, "负向对照：不回退则与上滚结果不同");
});

test("renderScreen：T05 末帧含 cwd 行（修前缺失，负向对照）", () => {
  const raw = fs.readFileSync(path.join(REPO, "golden-tui/T05/run-1/raw.bin"));
  const off = lastSnapshotOffset(path.join(REPO, "golden-tui/T05/run-1/meta.json"));
  const snapped = snapOffsetToFrameEnd(raw, off);
  const text = normalizeScreen(renderScreen(trimRawToOffset(raw, snapped ?? raw.length), 100, 30)).join("\n");
  assert.ok(text.includes("/work"), "修复后 T05 末帧必须含 cwd 行（vt100 同输入有该行）");
  assert.ok(text.includes("/128k (auto)"), "用量段仍在");
});
