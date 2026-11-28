// tools/difftest/test/primitives.test.mjs —— 每个脚本原语的 4 类行为。
//
// 契约（RUNBOOK-4 / P3.-2）：条件按时满足 / 永不满足时超时失败 /
// 条件满足之前输出流关闭时失败 / 子进程提前退出时失败。
// 依赖全部注入，不启动 TS pi。

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  EventQueue,
  createFile,
  releaseGate,
  send,
  waitEvent,
  waitExit,
  waitIdle,
  waitMockConsumed,
  waitState,
} from "../primitives.mjs";

const TIMEOUT = 40;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/// 假子进程：只提供 stdin.write 与 exited。
function fakeChild({ exitedMs = null } = {}) {
  const written = [];
  return {
    written,
    stdin: { write: (chunk) => written.push(chunk) },
    exited:
      exitedMs === null
        ? new Promise(() => {}) // 永不结束
        : sleep(exitedMs).then(() => "closed"),
  };
}

/// 把"流关闭 / 子进程退出"表达成队列的 failAll（run.mjs 就是这么接的）。
function killQueue(queue, reason) {
  queue.failAll(reason);
}

// --- waitEvent -------------------------------------------------------------

test("waitEvent：按时满足", async () => {
  const queue = new EventQueue(TIMEOUT);
  const pending = waitEvent({ queue, match: '"type":"x"' });
  queue.push('{"type":"x"}');
  assert.equal(await pending, '{"type":"x"}');
});

test("waitEvent：永不满足则超时失败", async () => {
  const queue = new EventQueue(TIMEOUT);
  queue.push('{"type":"other"}');
  await assert.rejects(waitEvent({ queue, match: '"type":"x"' }), /超时/);
});

test("waitEvent：满足前输出流关闭则失败", async () => {
  const queue = new EventQueue(5000);
  const pending = waitEvent({ queue, match: '"type":"x"' });
  killQueue(queue, "子进程输出流已关闭");
  await assert.rejects(pending, /输出流已关闭/);
});

test("waitEvent：子进程提前退出则失败", async () => {
  const queue = new EventQueue(5000);
  const pending = waitEvent({ queue, match: '"type":"x"' });
  killQueue(queue, "子进程已退出");
  await assert.rejects(pending, /子进程已退出/);
});

// --- send ------------------------------------------------------------------

test("send：命令被写入且响应按时到达", async () => {
  const queue = new EventQueue(TIMEOUT);
  const child = fakeChild();
  const pending = send({ queue, child, command: { type: "prompt" } });
  assert.equal(child.written.length, 1);
  assert.match(child.written[0], /"type":"prompt"/);
  queue.push('{"type":"response","command":"prompt","success":true}');
  assert.match(await pending, /"success":true/);
});

test("send：响应永不到达则超时失败", async () => {
  const queue = new EventQueue(TIMEOUT);
  const child = fakeChild();
  await assert.rejects(send({ queue, child, command: { type: "prompt" } }), /超时/);
});

test("send：响应到达前输出流关闭则失败", async () => {
  const queue = new EventQueue(5000);
  const child = fakeChild();
  const pending = send({ queue, child, command: { type: "prompt" } });
  killQueue(queue, "子进程输出流已关闭");
  await assert.rejects(pending, /输出流已关闭/);
});

test("send：响应到达前子进程退出则失败", async () => {
  const queue = new EventQueue(5000);
  const child = fakeChild();
  const pending = send({ queue, child, command: { type: "prompt" } });
  killQueue(queue, "子进程已退出");
  await assert.rejects(pending, /子进程已退出/);
});

// --- waitState -------------------------------------------------------------

test("waitState：命中期望状态后返回该行", async () => {
  const queue = new EventQueue(TIMEOUT);
  const child = fakeChild();
  const pending = waitState({ queue, child, want: '"isStreaming":false' });
  queue.push('{"type":"response","command":"get_state","data":{"isStreaming":true}}');
  queue.push('{"type":"response","command":"get_state","data":{"isStreaming":false}}');
  assert.match(await pending, /"isStreaming":false/);
  assert.equal(child.written.length, 2, "未命中时应继续轮询");
});

test("waitState：永不命中则超时失败", async () => {
  const queue = new EventQueue(TIMEOUT);
  const child = fakeChild();
  queue.push('{"type":"response","command":"get_state","data":{"isStreaming":true}}');
  await assert.rejects(waitState({ queue, child, want: '"isStreaming":false' }), /超时/);
});

test("waitState：命中前输出流关闭则失败", async () => {
  const queue = new EventQueue(5000);
  const child = fakeChild();
  const pending = waitState({ queue, child, want: "x" });
  killQueue(queue, "子进程输出流已关闭");
  await assert.rejects(pending, /输出流已关闭/);
});

test("waitState：命中前子进程退出则失败", async () => {
  const queue = new EventQueue(5000);
  const child = fakeChild();
  const pending = waitState({ queue, child, want: "x" });
  killQueue(queue, "子进程已退出");
  await assert.rejects(pending, /子进程已退出/);
});

// --- waitIdle --------------------------------------------------------------

test("waitIdle：等到的 agent_end 按时返回", async () => {
  const queue = new EventQueue(TIMEOUT);
  const pending = waitIdle({ queue });
  queue.push('{"type":"agent_end"}');
  assert.equal(await pending, '{"type":"agent_end"}');
});

test("waitIdle：fresh 时丢弃旧事件，避免匹配上一轮", async () => {
  const queue = new EventQueue(TIMEOUT);
  queue.push('{"type":"agent_end"}');
  const pending = waitIdle({ queue, fresh: true });
  queue.push('{"type":"agent_end"}');
  assert.equal(await pending, '{"type":"agent_end"}');
});

test("waitIdle：永不出现则超时失败", async () => {
  const queue = new EventQueue(TIMEOUT);
  await assert.rejects(waitIdle({ queue }), /超时/);
});

test("waitIdle：输出流关闭则失败", async () => {
  const queue = new EventQueue(5000);
  const pending = waitIdle({ queue });
  killQueue(queue, "子进程输出流已关闭");
  await assert.rejects(pending, /输出流已关闭/);
});

test("waitIdle：子进程退出则失败", async () => {
  const queue = new EventQueue(5000);
  const pending = waitIdle({ queue });
  killQueue(queue, "子进程已退出");
  await assert.rejects(pending, /子进程已退出/);
});

// --- waitMockConsumed ------------------------------------------------------

test("waitMockConsumed：达到目标后返回状态", async () => {
  const statuses = [{ consumed: 1, inFlight: 1 }, { consumed: 2, inFlight: 0 }];
  let index = 0;
  const status = await waitMockConsumed({
    mockStatus: async () => statuses[Math.min(index++, statuses.length - 1)],
    n: 2,
    timeoutMs: 500,
    sleep,
  });
  assert.equal(status.consumed, 2);
});

test("waitMockConsumed：永不达到则超时失败", async () => {
  await assert.rejects(
    waitMockConsumed({
      mockStatus: async () => ({ consumed: 0, inFlight: 0 }),
      n: 2,
      timeoutMs: TIMEOUT,
      sleep,
    }),
    /超时/,
  );
});

test("waitMockConsumed：mock 查询抛错时原样失败", async () => {
  await assert.rejects(
    waitMockConsumed({
      mockStatus: async () => {
        throw new Error("mock 已退出");
      },
      n: 1,
      timeoutMs: 500,
      sleep,
    }),
    /mock 已退出/,
  );
});

test("waitMockConsumed：inFlight 未归零时不提前返回", async () => {
  let calls = 0;
  await assert.rejects(
    waitMockConsumed({
      mockStatus: async () => {
        calls += 1;
        return { consumed: 5, inFlight: 1 };
      },
      n: 1,
      timeoutMs: TIMEOUT,
      sleep,
    }),
    /超时/,
  );
  assert.ok(calls > 1, "应持续轮询而不是提前返回");
});

// --- releaseGate -----------------------------------------------------------

test("releaseGate：2xx 时返回响应", async () => {
  const response = await releaseGate({
    fetchImpl: async () => ({ ok: true, status: 200 }),
    port: 1,
  });
  assert.equal(response.status, 200);
});

test("releaseGate：非 2xx 时失败（注入没生效要暴露）", async () => {
  await assert.rejects(
    releaseGate({ fetchImpl: async () => ({ ok: false, status: 500 }), port: 1 }),
    /HTTP 500/,
  );
});

test("releaseGate：fetch 抛错时失败", async () => {
  await assert.rejects(
    releaseGate({
      fetchImpl: async () => {
        throw new Error("连接被拒");
      },
      port: 1,
    }),
    /连接被拒/,
  );
});

test("releaseGate：fetch 挂住时超时失败", async () => {
  await assert.rejects(
    releaseGate({
      fetchImpl: () => new Promise(() => {}),
      port: 1,
      timeoutMs: TIMEOUT,
    }),
    /超时/,
  );
});

// --- waitExit --------------------------------------------------------------

test("waitExit：进程按时结束时返回", async () => {
  assert.equal(await waitExit({ exited: sleep(5).then(() => "closed"), timeoutMs: 500 }), "closed");
});

test("waitExit：进程不结束则超时失败", async () => {
  await assert.rejects(
    waitExit({ exited: new Promise(() => {}), timeoutMs: TIMEOUT }),
    /超时/,
  );
});

test("waitExit：进程以异常方式结束（promise reject）时失败", async () => {
  await assert.rejects(
    waitExit({
      exited: sleep(5).then(() => {
        throw new Error("子进程被信号终止");
      }),
      timeoutMs: 500,
    }),
    /被信号终止/,
  );
});

test("waitExit：已结束的进程立即返回", async () => {
  assert.equal(await waitExit({ exited: Promise.resolve(0), timeoutMs: 500 }), 0);
});

// --- createFile ------------------------------------------------------------

test("createFile：写文件并返回路径", async () => {
  const written = [];
  const target = await createFile({
    fsImpl: { writeFile: async (p, c) => written.push([p, c]) },
    dir: "/tmp/x",
    path: "flag.txt",
    content: "1",
  });
  assert.equal(target, "/tmp/x/flag.txt");
  assert.deepEqual(written, [["/tmp/x/flag.txt", "1"]]);
});

test("createFile：缺省内容写空串", async () => {
  const written = [];
  await createFile({
    fsImpl: { writeFile: async (p, c) => written.push([p, c]) },
    dir: "/tmp/x",
    path: "flag.txt",
  });
  assert.deepEqual(written, [["/tmp/x/flag.txt", ""]]);
});

test("createFile：写失败时失败（目录不存在等）", async () => {
  await assert.rejects(
    createFile({
      fsImpl: {
        writeFile: async () => {
          throw new Error("ENOENT: 目录不存在");
        },
      },
      dir: "/tmp/missing",
      path: "flag.txt",
    }),
    /ENOENT/,
  );
});

test("createFile：目标已存在时覆盖而不是失败", async () => {
  const written = [];
  await createFile({
    fsImpl: { writeFile: async (p, c) => written.push([p, c]) },
    dir: "/tmp/x",
    path: "flag.txt",
    content: "second",
  });
  assert.equal(written[0][1], "second");
});
