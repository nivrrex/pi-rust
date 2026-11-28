#!/usr/bin/env node
// F57/C2：`/allcopy` 依赖的 shim `copyToClipboard` 的行为用例（`node --test`）。
//
// 判据全部来自 ref `utils/clipboard.ts:74-160` 与 `utils/clipboard-command.ts:5-53`：
//   1. Linux 有 `DISPLAY`、`xclip` 可用 ⇒ 走 `xclip -selection clipboard` 并把正文写进 stdin；
//   2. `xclip` 不存在 ⇒ 依次退到 `xsel`；都不存在且无远程/headless ⇒ 抛 X11 提示；
//   3. 远程会话（`SSH_CONNECTION`）⇒ OSC 52（`ESC ] 52 ; c ; <base64> BEL`）；
//   4. headless（无 DISPLAY/WAYLAND/TERMUX）且命令全失败 ⇒ OSC 52；
//   5. base64 超过 100_000 ⇒ 抛 `text exceeds the OSC 52 size limit`；
//   6. 写命令**非零退出** ⇒ 视为失败并继续回退（`undefined` 语义）。
//
// 做法：造一个只含假命令的临时 `PATH`（`xclip`/`xsel`/`wl-copy` 都是 shell 脚本，
// 把 stdin 落到文件），因此测的是**真实 spawn 路径 + 真实 stdin 字节**，不 mock 内部函数。

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

// F60/1b：**不要**在这里预置 `globalThis.require`。
//
// 真实宿主 `tools/ext-host/runner.mjs` 用 `await import(extensionImportUrl(...))`
// 加载扩展（ESM），**从不**注入 CommonJS 的 `require`。此前的测试为了让
// `shim` 里残留的 `require("node:child_process")` 通过，先设了
// `globalThis.require = createRequire(import.meta.url)` —— 这正是"测试通过、
// 真机报 `require is not defined`"的原因。
//
// 现在测试与真实宿主走**同一条加载路径**：直接 `import()`，不提供任何
// CommonJS 垫片。这样只要 shim 里再出现 `require`/`__dirname`/`module`，
// 测试就会像真机一样失败。
if (typeof globalThis.require !== "undefined") {
  throw new Error("测试环境不应存在 globalThis.require（会掩盖 ESM 下的 require 误用）");
}

const shim = await import("../shim/pi-coding-agent.mjs");

/** 造一个临时 bin 目录，放入给定脚本（文件名 → 脚本正文）。 */
function fakeBin(scripts) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "f57-bin-"));
  for (const [name, body] of Object.entries(scripts)) {
    const file = path.join(dir, name);
    fs.writeFileSync(file, `#!/bin/sh\n${body}\n`, { mode: 0o755 });
  }
  return dir;
}

/** 记录 stdout 的假流。 */
function capture() {
  const chunks = [];
  return { chunks, write: (text) => (chunks.push(text), true), text: () => chunks.join("") };
}

/**
 * 只带假 bin 的最小 env（清掉 DISPLAY 等，避免命中真实剪贴板）。
 *
 * `PATH` 末尾必须保留系统目录：假命令是 shell 脚本，内部要用 `cat` 等基础命令，
 * 只给假 bin 目录会让它们自己先失败（实测 `cat: 未找到命令` ⇒ 假命令 exit 127
 * ⇒ 被当成"剪贴板命令失败"，用例误判）。真实剪贴板命令在系统目录里，但我们的
 * 场景要么清掉了 `DISPLAY`/`WAYLAND_DISPLAY`（命令不会被选中），要么测试用的是
 * 假命令同名文件（`PATH` 前缀优先）。
 */
function baseEnv(bin, extra = {}) {
  return { PATH: `${bin}:/usr/local/bin:/usr/bin:/bin`, HOME: os.tmpdir(), ...extra };
}

test("xclip receives the exact stdin bytes", async () => {
  const out = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "f57-out-")), "clip.txt");
  const bin = fakeBin({ xclip: `cat > "${out}"` });
  const text = "第一行\n第二行 with space\tand tab\n";
  await shim.copyToClipboard(text, {
    platform: "linux",
    env: baseEnv(bin, { DISPLAY: ":0" }),
    stream: capture(),
  });
  assert.equal(fs.readFileSync(out, "utf8"), text, "stdin 必须逐字节等于正文");
});

test("falls back to xsel when xclip fails", async () => {
  const out = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "f57-out-")), "clip.txt");
  // 系统里可能**真的**装了 `xclip`（`PATH` 尾部保留系统目录），所以"缺失"用
  // "同名假命令直接失败"来表达——与 ref 的 `undefined` 语义等价。
  const bin = fakeBin({ xclip: "exit 1", xsel: `cat > "${out}"` });
  await shim.copyToClipboard("via-xsel", {
    platform: "linux",
    env: baseEnv(bin, { DISPLAY: ":0" }),
    stream: capture(),
  });
  assert.equal(fs.readFileSync(out, "utf8"), "via-xsel");
});

test("non-zero exit from the writer counts as failure and keeps falling back", async () => {
  const out = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "f57-out-")), "clip.txt");
  // xclip 失败（exit 1），xsel 成功 ⇒ 正文必须由 xsel 收到。
  const bin = fakeBin({ xclip: "exit 1", xsel: `cat > "${out}"` });
  await shim.copyToClipboard("after-failure", {
    platform: "linux",
    env: baseEnv(bin, { DISPLAY: ":0" }),
    stream: capture(),
  });
  assert.equal(fs.readFileSync(out, "utf8"), "after-failure");
});

test("no usable writer with DISPLAY throws the X11 hint", async () => {
  // `DISPLAY` 指向不存在的 X server：系统里的 `xclip` 会自己失败（exit ≠ 0），
  // 等价于"装了但不可用"——这正是 ref 报 X11 提示的那条路径。
  const bin = fakeBin({});
  await assert.rejects(
    () =>
      shim.copyToClipboard("x", {
        platform: "linux",
        env: baseEnv(bin, { DISPLAY: ":99999" }),
        stream: capture(),
      }),
    /install `xclip` or `xsel`/,
  );
});

test("headless without commands emits OSC 52", async () => {
  const bin = fakeBin({});
  const stream = capture();
  await shim.copyToClipboard("hello", { platform: "linux", env: baseEnv(bin), stream });
  const expected = `\u001b]52;c;${Buffer.from("hello", "utf8").toString("base64")}\u0007`;
  assert.equal(stream.text(), expected);
});

test("remote session emits OSC 52 even with a display", async () => {
  const bin = fakeBin({});
  const stream = capture();
  await shim.copyToClipboard("remote", {
    platform: "linux",
    env: baseEnv(bin, { DISPLAY: ":0", SSH_CONNECTION: "1 2 3 4" }),
    stream,
  });
  assert.match(stream.text(), /^\u001b\]52;c;[A-Za-z0-9+/=]+\u0007$/);
});

test("oversized payload reports the OSC 52 limit error", async () => {
  const bin = fakeBin({});
  await assert.rejects(
    () => shim.copyToClipboard("x".repeat(200_000), { platform: "linux", env: baseEnv(bin), stream: capture() }),
    /text exceeds the OSC 52 size limit/,
  );
});

test("wayland uses wl-copy", async () => {
  const out = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "f57-out-")), "clip.txt");
  const bin = fakeBin({ "wl-copy": `cat > "${out}"` });
  await shim.copyToClipboard("wayland", {
    platform: "linux",
    env: baseEnv(bin, { WAYLAND_DISPLAY: "wayland-0" }),
    stream: capture(),
  });
  assert.equal(fs.readFileSync(out, "utf8"), "wayland");
});
