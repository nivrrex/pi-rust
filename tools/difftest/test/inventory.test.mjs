//! 场景清单对账的测试。重点在**负向对照**：必须确认对账真的会失败，
//! 而不是恒真（"测试通过 ≠ 被测路径真的走到了"）。

import { test } from "node:test";
import assert from "node:assert/strict";

import { collectResultRows, formatReconciliation, listScenarios, reconcile } from "../inventory.mjs";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

test("scenarios/ 的清单是从目录机械列出来的，不是写死的", async () => {
  const scenarios = await listScenarios();
  assert.ok(scenarios.length >= 40, `场景数应 >= 40，实际 ${scenarios.length}`);
  // 目录里真实存在的几个必须在内；这条断言在有人删场景时会失败，正是想要的。
  // S13b/S13c 是暴露过覆盖面缺口的两个（我此前用的是派生场景 S13b-text/S13b-toolcall）；
  // S18/X01/X02 尚未创建（属 P4-1），清单对账会在它们落地后自动要求它们入表。
  for (const name of ["S01", "S13b", "S13c"]) {
    assert.ok(scenarios.includes(name), `${name} 应在 scenarios/ 清单里`);
  }
  // 去重且有序
  assert.deepEqual(scenarios, [...new Set(scenarios)].sort());
});

test("一一对应时 ok", () => {
  const result = reconcile(["S01", "S02"], ["S01", "S02"]);
  assert.equal(result.ok, true);
  assert.deepEqual(result.missing, []);
  assert.deepEqual(result.extra, []);
});

test("负向对照：少一个场景必须失败，且把名字列出来", () => {
  const result = reconcile(["S01", "S02", "S03"], ["S01", "S03"]);
  assert.equal(result.ok, false);
  assert.deepEqual(result.missing, ["S02"]);
  const text = formatReconciliation(result, ["S01", "S02", "S03"]);
  assert.match(text, /S02/);
  assert.match(text, /对账失败/);
});

test("负向对照：结果表里多一个未知场景也必须失败", () => {
  const result = reconcile(["S01"], ["S01", "S99"]);
  assert.equal(result.ok, false);
  assert.deepEqual(result.extra, ["S99"]);
});

test("负向对照：完全空的结果表必须列出全部缺失", () => {
  const result = reconcile(["S01", "S02"], []);
  assert.equal(result.ok, false);
  assert.equal(result.missing.length, 2);
});

test("collectResultRows 忽略 `_` 开头的运行记录目录", async () => {
  const dir = await mkdtemp(path.join(tmpdir(), "inv-"));
  await mkdir(path.join(dir, "S01"));
  await mkdir(path.join(dir, "_runs"));
  await writeFile(path.join(dir, "not-a-dir"), "x");
  const rows = await collectResultRows(dir);
  assert.deepEqual(rows, ["S01"]);
});

test("负向对照：产出目录缺一个场景时，端到端对账必须失败", async () => {
  const dir = await mkdtemp(path.join(tmpdir(), "inv-"));
  const scenarios = await listScenarios();
  // 造出"全部场景都有"的目录：必须一一对应
  for (const name of scenarios) await mkdir(path.join(dir, name));
  const complete = reconcile(scenarios, await collectResultRows(dir));
  assert.equal(complete.ok, true, "全部场景都在时应当 ok");
  // 删掉一个目录后必须失败（说明对账看的是真实目录，不是我传进去的期望值）
  const { rm } = await import("node:fs/promises");
  await rm(path.join(dir, scenarios[0]), { recursive: true });
  const incomplete = reconcile(scenarios, await collectResultRows(dir));
  assert.equal(incomplete.ok, false);
  assert.deepEqual(incomplete.missing, [scenarios[0]]);
});
