// tools/difftest/primitives.mjs —— 等待原语的事件队列，抽成独立模块以便自测。
//
// 抽出来的原因见 notes/questions.md 的 Q14：`push` 原先把**任何**新行交给最老的等待者，
// 完全不检查匹配条件，于是 `waitEvent("永不出现的事件")` 会被下一条任意行唤醒。
//
// 契约：任何等待只能以三种方式结束——条件满足 / 超时失败 / 子进程提前退出失败；
// 后两种必须让整个运行失败，不得返回"成功"。

/// 把场景里的 `match` 片段（形如 `"type":"text_start"`）解析成 `{type: "text_start"}`。
///
/// 解析失败返回 `null`，调用方退回子串匹配。
function parseMatchFragment(match) {
  try {
    const value = JSON.parse(`{${match}}`);
    return value !== null && typeof value === "object" && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

/// 行解析成对象；失败返回 `null`。
function parseLine(line) {
  try {
    const value = JSON.parse(line);
    return value !== null && typeof value === "object" && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

/// 行的字段是否与 `match` 片段逐字段相等。
///
/// **按解析后的 JSON 字段比较**，只有在行或片段无法解析时才退回子串匹配——
/// 这样"正文里恰好含有目标串"不会造成误匹配（有反例测试）。
export function matchesLine(line, match) {
  const fragment = parseMatchFragment(match);
  const parsed = fragment === null ? null : parseLine(line);
  if (fragment === null || parsed === null) {
    return line.includes(match);
  }
  return Object.entries(fragment).every(([key, want]) => {
    const got = parsed[key];
    if (want !== null && typeof want === "object") {
      return JSON.stringify(got) === JSON.stringify(want);
    }
    return got === want;
  });
}

export class EventQueue {
  /// `timeoutMs` 是等待原语的默认超时；自测会传一个很短的值。
  ///
  /// 必须显式初始化——`#matched` 用它起定时器，漏掉它会让 `setTimeout(fn, undefined)`
  /// 变成 0ms 立即超时（P3.-2 第 1 条失败的根因）。
  constructor(timeoutMs = 60000) {
    this.lines = [];
    this.waiters = [];
    this.timeoutMs = timeoutMs;
  }
  push(line) {
    // **只**把匹配的行交给等待者。此前是无条件交给最老的等待者，
    // 于是 `waitEvent("永不出现的事件")` 会被下一条任意行唤醒——
    // 这是 Q14 的根因：等待原语的匹配条件根本没被检查。
    const index = this.waiters.findIndex((waiter) => waiter.any || matchesLine(line, waiter.match));
    if (index >= 0) {
      const [waiter] = this.waiters.splice(index, 1);
      waiter.resolve(line);
    } else {
      this.lines.push(line);
    }
  }
  discard() {
    this.lines = [];
  }
  take(match, label) {
    if (match === undefined) {
      // 不指定匹配条件：取缓冲区里的下一条，没有就等——**带超时**，
      // 否则会一直挂着（此前这里引用了 run.mjs 的 withTimeout，本模块里没有它，
      // 一走到这条分支就抛 ReferenceError）。
      const line = this.lines.shift();
      if (line !== undefined) return Promise.resolve(line);
      return new Promise((resolve, reject) => {
        const timer = setTimeout(
          () => reject(new Error(`${label ?? "取下一条事件"} 超时，属于失败保护被触发`)),
          this.timeoutMs,
        );
        this.waiters.push({
          match: "",
          resolve: (value) => {
            clearTimeout(timer);
            resolve(value);
          },
          reject: (error) => {
            clearTimeout(timer);
            reject(error);
          },
          // 无条件等待：任何行都算满足。
          any: true,
        });
      });
    }
    // 缓冲区的取用同样按解析后的字段比较，与 push 的口径一致。
    const index = this.lines.findIndex((l) => matchesLine(l, match));
    if (index >= 0) return Promise.resolve(this.lines.splice(index, 1)[0]);
    return this.#matched(match);
  }
  #next() {
    return new Promise((resolve) => this.waiters.push({ resolve }));
  }
  /// 子进程提前退出或输出流关闭时调用：所有在等的原语一律失败，不得当作满足。
  failAll(reason) {
    for (const waiter of this.waiters.splice(0)) {
      waiter.reject(new Error(`${reason}（原语在等：${waiter.match}）`));
    }
  }

  #matched(match) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`等待事件 ${match} 超时，属于失败保护被触发`)),
        this.timeoutMs,
      );
      const waiter = {
        match,
        resolve: (line) => {
          clearTimeout(timer);
          resolve(line);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      };
      this.waiters.push(waiter);
    });
  }
}

// ---------------------------------------------------------------------------
// 脚本原语的**可测形态**：把依赖（队列、子进程、mock 状态、fs、fetch）注入进来，
// 于是自测不必启动 TS pi，也能覆盖每一条失败路径。
//
// 契约统一：每个原语只能以三种方式结束——条件满足、超时失败、依赖提前终止（失败）。
// ---------------------------------------------------------------------------

/// 等待一个事件（`waitEvent`）。
export function waitEvent({ queue, match }) {
  return queue.take(match, `等待事件 ${match}`);
}

/// 发一条命令并等它的响应（`send`）。
export function send({ queue, child, command }) {
  child.stdin.write(`${JSON.stringify(command)}\n`);
  return queue.take(`"command":"${command.type}"`, `等待 ${command.type} 的响应`);
}

/// 以 `get_state` 轮询直到指定字段命中（`waitState`）。
///
/// 返回命中的那一行——调用方负责把它补进事件流（轮询次数与迟到响应都不应计入 B 层）。
export async function waitState({ queue, child, want }) {
  for (;;) {
    child.stdin.write(`${JSON.stringify({ type: "get_state" })}\n`);
    const line = await queue.take('"command":"get_state"', "等待 get_state 响应");
    if (line.includes(want)) return line;
  }
}

/// 等 `agent_end`（`waitIdle`）；`fresh` 时先丢弃已缓冲的旧事件。
export async function waitIdle({ queue, fresh }) {
  if (fresh) queue.discard();
  return queue.take('"type":"agent_end"', "等待 agent_end");
}

/// 轮询 mock 的 `consumed`/`inFlight`（`waitMockConsumed`）。
export async function waitMockConsumed({ mockStatus, n, timeoutMs = 60000, sleep }) {
  const startedAt = Date.now();
  for (;;) {
    const status = await mockStatus();
    if (status.consumed >= n && status.inFlight === 0) return status;
    if (Date.now() - startedAt >= timeoutMs) {
      throw new Error(`等待 mock 消费 ${n} 个响应 超时（${timeoutMs}ms），属于失败保护被触发`);
    }
    await sleep(5);
  }
}

/// 放行 mock 的闸门（`releaseGate`）。要求拿到 2xx——否则注入其实没生效。
export async function releaseGate({ fetchImpl, port, timeoutMs = 10000 }) {
  const response = await withTimeoutMs(
    fetchImpl(`http://127.0.0.1:${port}/control/release`, { method: "POST" }),
    "放行闸门",
    timeoutMs,
  );
  if (!response.ok) throw new Error(`放行闸门失败：HTTP ${response.status}`);
  return response;
}

/// 等子进程真正结束（`waitExit`）。`exited` 是 run.mjs 里 `child.on("close")` 的 promise。
export function waitExit({ exited, timeoutMs = 60000 }) {
  return withTimeoutMs(exited, "等待进程退出", timeoutMs);
}

/// 写一个标志文件（`createFile`）。
export async function createFile({ fsImpl, dir, path: relative, content }) {
  const target = `${dir}/${relative}`;
  await fsImpl.writeFile(target, content ?? "");
  return target;
}

/// 与 run.mjs 里同名的超时包装，独立一份以免跨模块依赖。
export function withTimeoutMs(promise, what, ms) {
  return Promise.race([
    promise,
    new Promise((_, reject) =>
      setTimeout(() => reject(new Error(`${what} 超时（${ms}ms），属于失败保护被触发`)), ms),
    ),
  ]);
}
