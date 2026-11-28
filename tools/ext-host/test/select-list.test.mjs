#!/usr/bin/env node
// F108：shim 的 `SelectList.render()` 必须使用构造时传入的**样式函数**
// （ref `packages/tui/src/components/select-list.ts:75-109` 与 `:183-215`）。
//
// `pi-interact`（用户已装）的多选面板传了 `selectedPrefix`/`selectedText`/
// `description`/`scrollInfo`/`noMatch` 五个，此前 shim 的 render **全部忽略** ⇒
// 面板没有高亮色、没有描述列、没有滚动提示。
import { test } from "node:test";
import assert from "node:assert/strict";

import { SelectList } from "../shim/pi-tui.mjs";

const tag = (name) => (text) => `<${name}>${text}</${name}>`;

function makeList(overrides = {}) {
  return new SelectList(
    [
      { value: "a", label: "alpha", description: "first" },
      { value: "b", label: "beta", description: "second" },
    ],
    10,
    {
      selectedPrefix: tag("sel-prefix"),
      selectedText: tag("sel-text"),
      description: tag("desc"),
      scrollInfo: tag("scroll"),
      noMatch: tag("nomatch"),
      ...overrides,
    },
  );
}

test("selected item uses selectedText, unselected does not", () => {
  const lines = makeList().render(80);
  assert.ok(lines[0].includes("<sel-text>"), `选中行应套 selectedText：${lines[0]}`);
  assert.ok(lines[1].includes("beta"), `未选中行应在：${lines[1]}`);
  assert.ok(!lines[1].includes("<sel-text>"), `未选中行不该套 selectedText：${lines[1]}`);
});

test("description column appears when wide enough", () => {
  const lines = makeList().render(80);
  const joined = lines.join("\n");
  assert.ok(joined.includes("<desc>first</desc>"), `应渲染描述列：${joined}`);
});

test("empty list renders the noMatch text", () => {
  const list = new SelectList([], 10, { noMatch: tag("nomatch") });
  const lines = list.render(80);
  assert.equal(lines.length, 1, `${lines}`);
  assert.ok(lines[0].includes("<nomatch>"), `空列表应走 noMatch：${lines[0]}`);
});

test("scroll indicator appears when items exceed maxVisible", () => {
  const items = Array.from({ length: 20 }, (_, i) => ({ value: `v${i}`, label: `item${i}` }));
  const list = new SelectList(items, 5, { scrollInfo: tag("scroll") });
  const lines = list.render(80);
  const joined = lines.join("\n");
  assert.ok(joined.includes("<scroll>"), `应出现滚动提示：${joined}`);
  assert.ok(/\(\d+\/20\)/.test(joined), `滚动提示应含 (n/20)：${joined}`);
});

test("selection moves with handleInput and stays within bounds", () => {
  const list = makeList();
  assert.equal(list.getSelectedItem().value, "a");
  // `handleInput` 收的是**终端字节序列**（ref `keys.ts` 的 `Key.down` = `\x1b[B`），
  // 不是键名——`matchesKey` 按序列表判断。
  const DOWN = "\u001b[B";
  const UP = "\u001b[A";
  list.handleInput(DOWN);
  assert.equal(list.getSelectedItem().value, "b");
  list.handleInput(DOWN); // 已到末尾，不应越界
  assert.equal(list.getSelectedItem().value, "b");
  list.handleInput(UP);
  assert.equal(list.getSelectedItem().value, "a");
});
