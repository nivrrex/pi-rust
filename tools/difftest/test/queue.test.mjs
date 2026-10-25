// tools/difftest/test/queue.test.mjs —— 等待原语的负向对照自测（node --test，无新依赖）。
//
// 背景（Q14）：`push` 原先把**任何**新行交给最老的等待者，不检查匹配条件，
// 于是 `waitEvent("永不出现的事件")` 会被下一条任意行唤醒、等待原语形同虚设。
//
// 契约：任何等待只能以三种方式结束——条件满足 / 超时失败 / 子进程提前退出失败。
// 后两种必须让运行失败，不得返回"成功"。

import { test } from "node:test";
import assert from "node:assert/strict";

import { EventQueue } from "../primitives.mjs";

test("条件按时满足：只把匹配的行交给等待者", async () => {
  const queue = new EventQueue(30);
  const pending = queue.take('"type":"wanted"', "等待 wanted");
  // 不匹配的行不得唤醒等待者，而应留在缓冲里。
  queue.push('{"type":"unwanted"}');
  let settled = false;
  pending.then(() => {
    settled = true;
  });
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(settled, false, "不匹配的行不得唤醒等待者");

  queue.push('{"type":"wanted"}');
  assert.equal(await pending, '{"type":"wanted"}');
});

test("永不满足：超时失败，且错误信息写明在等什么", async () => {
  const queue = new EventQueue(30);
  const pending = queue.take('"type":"never"', "等待 never", 30);
  await assert.rejects(pending, /超时/);
  // 超时后不得再被后续行"补上"。
  queue.push('{"type":"wanted"}');
});

test("条件满足之前子进程退出：必须失败，不得当作满足", async () => {
  const queue = new EventQueue(30);
  const pending = queue.take('"type":"never"', "等待 never");
  queue.failAll("子进程已退出");
  await assert.rejects(pending, /子进程已退出/);
});

test("条件满足之前输出流关闭：必须失败", async () => {
  const queue = new EventQueue(30);
  const pending = queue.take('"type":"never"', "等待 never");
  queue.failAll("子进程输出流已关闭");
  await assert.rejects(pending, /输出流已关闭/);
});

test("缓冲里已有的匹配行会被立即取走", async () => {
  const queue = new EventQueue(30);
  queue.push('{"type":"wanted"}');
  assert.equal(await queue.take('"type":"wanted"', "等待 wanted"), '{"type":"wanted"}');
});

test("多个等待者各等各的条件，互不串线", async () => {
  const queue = new EventQueue(30);
  const a = queue.take('"type":"a"', "等待 a");
  const b = queue.take('"type":"b"', "等待 b");
  queue.push('{"type":"b"}');
  assert.equal(await b, '{"type":"b"}');
  queue.push('{"type":"a"}');
  assert.equal(await a, '{"type":"a"}');
});

test("failAll 之后新起的等待不受影响", async () => {
  const queue = new EventQueue(30);
  queue.failAll("先前的退出");
  const pending = queue.take('"type":"x"', "等待 x", 30);
  queue.push('{"type":"x"}');
  assert.equal(await pending, '{"type":"x"}');
});

test("反例：正文含目标串但事件类型不同，不得匹配", () => {
  const queue = new EventQueue(30);
  // 行里**含** `"type":"text_start"` 这段文本，但它是某个字段的**值**，不是事件类型。
  queue.push('{"type":"message_update","delta":"blah \\"type\\":\\"text_start\\" blah"}');
  // 缓冲里没有"事件类型为 text_start"的行，因此这个等待必须超时失败。
  return assert.rejects(queue.take('"type":"text_start"', "等待 text_start"), /超时/);
});

test("按字段比较复杂字段（对象）也成立", async () => {
  const queue = new EventQueue(30);
  queue.push('{"type":"x","data":{"a":1}}');
  assert.equal(await queue.take('"data":{"a":1}', "等待 data"), '{"type":"x","data":{"a":1}}');
});

test("行无法解析时退回子串匹配", async () => {
  const queue = new EventQueue(30);
  queue.push("这行不是 JSON，但含 wanted-字面量");
  assert.equal(
    await queue.take("wanted-字面量", "等待字面量"),
    "这行不是 JSON，但含 wanted-字面量",
  );
});

test("take(undefined) 取缓冲里的下一条（此前会抛 ReferenceError）", async () => {
  const queue = new EventQueue(30);
  queue.push('{"type":"a"}');
  assert.equal(await queue.take(undefined, "取下一条"), '{"type":"a"}');
});

test("take(undefined) 无缓冲时超时失败，而不是抛 ReferenceError", async () => {
  const queue = new EventQueue(30);
  await assert.rejects(queue.take(undefined, "取下一条"), /超时/);
});
