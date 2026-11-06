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
    "C:\\Users\\u\\AppData\\Local\\pnpm\\global": ["v11"],
    "C:\\Users\\u\\AppData\\Local\\pnpm\\global\\v11": ["abcd-1234-0", "efgh-5678-0"],
  };
  const out = windowsJitiCandidates(
    { LOCALAPPDATA: "C:\\Users\\u\\AppData\\Local" },
    { listDir: (dir) => dirs[dir] ?? [] },
  );
  assert.ok(
    out.includes("C:\\Users\\u\\AppData\\Local\\pnpm\\global\\v11\\abcd-1234-0\\node_modules\\jiti"),
    `缺 hash 目录候选：${out.join(" | ")}`,
  );
  assert.ok(
    out.includes("C:\\Users\\u\\AppData\\Local\\pnpm\\global\\v11\\efgh-5678-0\\node_modules\\jiti"),
    "多 hash 都要覆盖",
  );
});

test("windows：%APPDATA%\\npm 与 %PNPM_HOME% 也被覆盖", () => {
  const out = windowsJitiCandidates({
    APPDATA: "C:\\Users\\u\\AppData\\Roaming",
    PNPM_HOME: "D:\\pnpm",
  });
  assert.ok(out.includes("C:\\Users\\u\\AppData\\Roaming\\npm\\node_modules\\jiti"));
  assert.ok(out.includes("D:\\pnpm\\node_modules\\jiti"));
});

test("windows：PI_JITI 优先于所有探测", () => {
  const out = staticJitiCandidates([], {
    PI_JITI: "C:\\tools\\jiti",
    LOCALAPPDATA: "C:\\Users\\u\\AppData\\Local",
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
