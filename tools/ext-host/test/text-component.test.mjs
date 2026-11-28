#!/usr/bin/env node
// F109：shim 的 `Text.render()` 必须与 ref `components/text.ts:48-100` 一致。
//
// 三处此前不同：①空文本应返回**空数组**；②每行要加**左右两侧** padding 并补齐到整宽；
// ③`paddingY` 的空行是 **width 个空格**、不是空串。这些都会影响逐帧对拍。
import { test } from "node:test";
import assert from "node:assert/strict";

import { Text } from "../shim/pi-tui.mjs";

test("empty text renders nothing (ref returns an empty array)", () => {
  assert.deepEqual(new Text("", 1, 1).render(20), []);
  assert.deepEqual(new Text("   ", 1, 1).render(20), [], "纯空白也算空");
});

test("content lines are padded on the right to the full width", () => {
  const lines = new Text("ab", 1, 0).render(10);
  assert.equal(lines.length, 1, `${lines}`);
  // 左 1 + "ab" + 右 1 + 补到 10 宽。
  assert.equal(lines[0], ` ab${" ".repeat(7)}`);
});

test("paddingY rows are full-width spaces, not empty strings", () => {
  const lines = new Text("ab", 1, 2).render(10);
  assert.equal(lines.length, 5, "上 2 + 内容 1 + 下 2：" + JSON.stringify(lines));
  assert.equal(lines[0], " ".repeat(10));
  assert.equal(lines[4], " ".repeat(10));
});

test("paddingX shrinks when the line would not fit", () => {
  // ref `:64-65`：`paddingX = min(paddingX, max(0, floor((width-1)/2)))`。
  const lines = new Text("ab", 9, 0).render(10);
  assert.equal(lines.length, 1, `${lines}`);
  assert.ok(!lines[0].startsWith("         "), `paddingX 应被夹到 4 以内：${JSON.stringify(lines[0])}`);
  assert.equal(lines[0].length >= 10, true, "仍应补齐到至少整宽");
});

test("tabs become three spaces", () => {
  const lines = new Text("a\tb", 0, 0).render(20);
  assert.ok(lines[0].includes("a   b"), `制表符应换成 3 空格：${JSON.stringify(lines[0])}`);
});
