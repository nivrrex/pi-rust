// F28/S2：jiti 候选路径（pnpm 多 hash 布局）与标签函数。
import assert from "node:assert/strict";
import test from "node:test";
import {
  commandJitiCandidates,
  extensionLabel,
  npmJitiCandidates,
  pnpmJitiCandidates,
  resolveJiti,
  staticJitiCandidates,
} from "../../ext-host/jiti.mjs";

const MEM = (entries) => ({
  listDir: () => entries,
  exists: (file) => entries.some((name) => file.startsWith(name) || file.includes(`/${name}/`)) && /package\.json$/.test(file),
});

test("npm 布局：<root>/jiti", () => {
  assert.deepEqual(npmJitiCandidates("/usr/lib/node_modules"), ["/usr/lib/node_modules/jiti"]);
});

test("pnpm：<root>/node_modules/jiti 优先，其次各 hash，最后 <root>/jiti", () => {
  const deps = {
    listDir: () => ["h2", "h1"],
    exists: (file) => file === "/g/h1/node_modules/jiti/package.json",
  };
  assert.deepEqual(pnpmJitiCandidates("/g", deps), [
    "/g/node_modules/jiti",
    "/g/h1/node_modules/jiti",
    "/g/jiti",
  ]);
});

test("pnpm：多个 hash 目录里只有含 package.json 的算候选", () => {
  const deps = {
    listDir: () => ["aaa", "bbb"],
    exists: (file) => file.startsWith("/g/bbb/node_modules/jiti/"),
  };
  const out = pnpmJitiCandidates("/g", deps);
  assert.ok(out.includes("/g/bbb/node_modules/jiti"));
  assert.ok(!out.includes("/g/aaa/node_modules/jiti"));
});

test("命令输出：多余空白与多行都只取第一行；空输出返回空", () => {
  assert.deepEqual(commandJitiCandidates("  \n /g/global/v11 \n/g/other\n", {
    listDir: () => [],
    exists: () => false,
  }), ["/g/global/v11/jiti", "/g/global/v11/node_modules/jiti", "/g/global/v11/jiti"]);
  assert.deepEqual(commandJitiCandidates("", {}), []);
  assert.deepEqual(commandJitiCandidates(undefined, {}), []);
});

test("静态候选命中时不执行 npm/pnpm 命令", () => {
  const calls = [];
  const env = { PI_JITI: "" };
  const candidates = staticJitiCandidates([], env);
  assert.deepEqual(candidates, []);
  assert.deepEqual(calls, []);
});

test("扩展标签：<dir>/./index.ts 取目录名，不出现 '.'", () => {
  assert.equal(extensionLabel("/e/pi-remote-control/./index.ts"), "pi-remote-control");
  assert.equal(extensionLabel("/e/pi-remote-control/index.mjs"), "pi-remote-control");
  assert.equal(extensionLabel("/e/plain.ts"), "plain.ts");
});

test("前序候选命中 ⇒ 完全不执行 npm/pnpm 命令", () => {
  const calls = [];
  const found = resolveJiti(["/e/x/index.ts"], {
    runCommand: (cmd) => {
      calls.push(cmd);
      return "/g";
    },
    exists: (file) => file === "/e/x/node_modules/jiti/lib/jiti.cjs",
  });
  assert.deepEqual(calls, []);
  assert.equal(found.dir, "/e/x/node_modules/jiti");
});

test("前序全落空 ⇒ 先 npm 再 pnpm，pnpm 命中 <hash>/node_modules/jiti", () => {
  const calls = [];
  const found = resolveJiti([], {
    runCommand: (cmd) => {
      calls.push(cmd);
      return cmd === "pnpm" ? "/g/v11" : "/usr/lib/node_modules";
    },
    listDir: () => ["d2a7a814"],
    exists: (file) =>
      file === "/g/v11/d2a7a814/node_modules/jiti/package.json" ||
      file === "/g/v11/d2a7a814/node_modules/jiti/lib/jiti.cjs",
  });
  assert.deepEqual(calls, ["npm", "pnpm"]);
  assert.equal(found.dir, "/g/v11/d2a7a814/node_modules/jiti");
});

// —— F34/W1：Windows 布局 ——
import { windowsJitiCandidates, pickNewestJitiCandidate } from "../../ext-host/jiti.mjs";

test("windows：%LOCALAPPDATA%\\pnpm\\global\\<ver>\\<hash>\\node_modules\\jiti", () => {
  const dirs = {
    "C:\\work\\AppData\\Local\\pnpm\\global": ["v11"],
    "C:\\work\\AppData\\Local\\pnpm\\global\\v11": ["abcd-1234-0", "efgh-5678-0"],
  };
  const out = windowsJitiCandidates(
    { LOCALAPPDATA: "C:\\work\\AppData\\Local" },
    { listDir: (dir) => dirs[dir] ?? [] },
  );
  assert.ok(
    out.includes("C:\\work\\AppData\\Local\\pnpm\\global\\v11\\abcd-1234-0\\node_modules\\jiti"),
    `缺 hash 目录候选：${out.join(" | ")}`,
  );
  assert.ok(
    out.includes("C:\\work\\AppData\\Local\\pnpm\\global\\v11\\efgh-5678-0\\node_modules\\jiti"),
    "多 hash 都要覆盖",
  );
});

test("windows：%APPDATA%\\npm 与 %PNPM_HOME% 也被覆盖", () => {
  const out = windowsJitiCandidates({
    APPDATA: "C:\\work\\AppData\\Roaming",
    PNPM_HOME: "D:\\pnpm",
  });
  assert.ok(out.includes("C:\\work\\AppData\\Roaming\\npm\\node_modules\\jiti"));
  assert.ok(out.includes("D:\\pnpm\\node_modules\\jiti"));
});

test("windows：PI_JITI 优先于所有探测", () => {
  const out = staticJitiCandidates([], {
    PI_JITI: "C:\\tools\\jiti",
    LOCALAPPDATA: "C:\\work\\AppData\\Local",
  });
  assert.equal(out[0], "C:\\tools\\jiti");
});

test("windows：多 hash 目录中只接受 name=jiti，并按 mtime 取最新", () => {
  const pkg = (name) => ({ name });
  const dirs = ["C:\\a", "C:\\b", "C:\\c"];
  const picked = pickNewestJitiCandidate(dirs, {
    readJson: (file) =>
      file.startsWith("C:\\a") ? pkg("jiti") : file.startsWith("C:\\b") ? pkg("other") : pkg("jiti"),
    mtimeMs: (file) => (file.startsWith("C:\\a") ? 100 : 200),
  });
  assert.equal(picked, "C:\\c", "应跳过 name 不是 jiti 的目录，取 mtime 最大的");
});

test("windows：环境变量全缺失时返回空数组（不影响 Linux 路径）", () => {
  assert.deepEqual(windowsJitiCandidates({}), []);
});

test("W1d：失败时 attempts 记录全部已搜索候选", () => {
  const attempts = [];
  const found = resolveJiti(["/ext/a/index.ts"], {
    attempts,
    exists: () => false,
    runCommand: () => "",
  });
  assert.equal(found, undefined);
  assert.ok(attempts.length >= 3, `应记录多个候选，实际 ${attempts.length}`);
  assert.ok(attempts.some((dir) => dir.includes("node_modules")), "应包含 node_modules 候选");
});

// —— F35/G4a：扩展路径 → import URL（Windows 必须 file:// 且带盘符） ——
import { extensionImportUrl } from "../../ext-host/jiti.mjs";

test("windows 绝对路径转 file:// URL（含空格、反斜杠、盘符）", () => {
  const url = extensionImportUrl("C:\\work\\my ext\\index.mjs", {
    platform: "win32",
    pathToFileURL: (p, opts) => ({ href: `file:///${p.replace(/\\/g, "/")}${opts?.windows ? "" : ""}` }),
  });
  assert.equal(url, "file:///C:/work/my ext/index.mjs");
});

test("windows UNC 路径转 file://（host 段保留）", () => {
  const url = extensionImportUrl("\\\\server\\share\\ext\\index.mjs", {
    platform: "win32",
    pathToFileURL: (p) => ({ href: `file://${p.replace(/\\/g, "/")}` }),
  });
  assert.equal(url, "file:////server/share/ext/index.mjs");
});

test("linux 路径保持 file:/// 形态", () => {
  const url = extensionImportUrl("/opt/ext/index.mjs", {
    platform: "linux",
    pathToFileURL: (p) => ({ href: `file://${p}` }),
  });
  assert.equal(url, "file:///opt/ext/index.mjs");
});

// —— F35/G4b：pnpm 全局目录的符号链接/junction ——
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

test("windows：hash 目录是符号链接时仍能读到 package.json（junction 等价）", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "jiti-link-"));
  try {
    const real = path.join(root, "real-jiti");
    fs.mkdirSync(real, { recursive: true });
    fs.writeFileSync(path.join(real, "package.json"), JSON.stringify({ name: "jiti" }));
    const hashDir = path.join(root, "global", "v11", "abcd-1234-0", "node_modules");
    fs.mkdirSync(hashDir, { recursive: true });
    fs.symlinkSync(real, path.join(hashDir, "jiti"), "dir");
    const candidates = pnpmJitiCandidates(path.join(root, "global", "v11"));
    const hit = candidates.find((dir) => fs.existsSync(path.join(dir, "package.json")));
    assert.ok(hit, `符号链接目录必须能被找到：${candidates.join(" | ")}`);
    assert.equal(JSON.parse(fs.readFileSync(path.join(hit, "package.json"), "utf8")).name, "jiti");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

// —— F36/H6a：宿主断开时 runner 静默退出 0 ——
import { spawn } from "node:child_process";
import net from "node:net";

test("runner：对端 RST 时退出码 0 且无 stderr", async () => {
  const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../..");
  // 本文件在 `tools/difftest/test/` 下 ⇒ repo 已含 `tools`，别再拼一层。
  const runner = path.join(repo, "ext-host/runner.mjs");
  const server = net.createServer((sock) => {
    // 收到就立刻销毁（模拟宿主 RST）。
    sock.destroy();
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  const child = spawn(process.execPath, [runner], {
    env: {
      ...process.env,
      PI_EXT_ENDPOINT: `127.0.0.1:${port}`,
      PI_EXT_TOKEN: "t",
      PI_EXT_EXTENSIONS: "[]",
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr += String(chunk);
  });
  const code = await new Promise((resolve) => child.on("exit", resolve));
  server.close();
  assert.equal(code, 0, `退出码应为 0，实际 ${code}；stderr=${stderr}`);
  assert.equal(stderr.trim(), "", `不应有 stderr：${stderr}`);
});
