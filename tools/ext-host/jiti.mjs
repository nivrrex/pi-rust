// jiti 解析与扩展标签（F28/S2）。
//
// 纯函数 + 可注入依赖，便于单测；`runner.mjs` 只调用 `resolveJiti` / `extensionLabel`。
// 与 `pi-cli` 的 `extension_labels()` 同规则：`<dir>/./index.ts` ⇒ 目录名（不是 "."）。

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

// 探测顺序：PI_JITI → 扩展目录 node_modules/jiti → 已安装 pi 的 node_modules/jiti
// （路径来自 PI_PACKAGE_DIR 或 TS_PI_ENTRY，不硬编码用户目录）→ **全都落空时**才问
// `npm root -g` / `pnpm root -g`（F28/S2：命令有副作用/开销，前序命中就不执行）。

/** npm 全局布局：`<root>/jiti`。 */
export function npmJitiCandidates(root) {
  return [path.join(root, "jiti")];
}

/**
 * pnpm 全局布局（F28/S2b）：
 * `<root>/node_modules/jiti`、`<root>/<hash>/node_modules/jiti`（可能多个，逐个校验
 * `package.json` 存在，全部保留由 `resolveJiti` 取第一个有效的）、`<root>/jiti`。
 * `deps` 可注入（单测用）：`{ listDir(root), exists(p) }`。
 */
export function pnpmJitiCandidates(root, deps = {}) {
  const listDir = deps.listDir ?? ((dir) => fs.readdirSync(dir));
  const exists = deps.exists ?? ((file) => fs.existsSync(file));
  const out = [path.join(root, "node_modules", "jiti")];
  let entries = [];
  try {
    entries = listDir(root).slice().sort();
  } catch {
    entries = [];
  }
  for (const name of entries) {
    const dir = path.join(root, name, "node_modules", "jiti");
    if (exists(path.join(dir, "package.json"))) out.push(dir);
  }
  out.push(path.join(root, "jiti"));
  return out;
}

/**
 * Windows 安装布局（F34/W1）：pnpm 全局目录在 Windows 上是
 * `%LOCALAPPDATA%\pnpm\global\<ver>\<hash>\node_modules\jiti`，
 * 另有 `%PNPM_HOME%`、`%APPDATA%\npm\node_modules\jiti`、以及 node.exe 同目录的
 * `node_modules\jiti`。**纯函数**：入参是环境变量表与注入的 fs 依赖，便于在 Linux 上单测。
 */
export function windowsJitiCandidates(env = {}, deps = {}) {
  const listDir = deps.listDir ?? ((dir) => fs.readdirSync(dir));
  const exists = deps.exists ?? ((file) => fs.existsSync(file));
  const join = deps.join ?? ((...parts) => parts.join("\\"));
  const out = [];
  const push = (dir) => {
    if (dir) out.push(dir);
  };

  // `%LOCALAPPDATA%\pnpm\global\<ver>\<hash>\node_modules\jiti`
  if (env.LOCALAPPDATA) {
    push(join(env.LOCALAPPDATA, "pnpm", "global", "node_modules", "jiti"));
    const globalRoot = join(env.LOCALAPPDATA, "pnpm", "global");
    let versions = [];
    try {
      versions = listDir(globalRoot).slice().sort();
    } catch {
      versions = [];
    }
    for (const version of versions) {
      const versionDir = join(globalRoot, version);
      push(join(versionDir, "node_modules", "jiti"));
      let hashes = [];
      try {
        hashes = listDir(versionDir).slice().sort();
      } catch {
        hashes = [];
      }
      for (const hash of hashes) {
        push(join(versionDir, hash, "node_modules", "jiti"));
      }
    }
  }
  // `%PNPM_HOME%`（pnpm 自己的根，可能直接就是 global 目录）
  if (env.PNPM_HOME) {
    push(join(env.PNPM_HOME, "node_modules", "jiti"));
    push(join(env.PNPM_HOME, "jiti"));
  }
  // `%APPDATA%\npm\node_modules\jiti`
  if (env.APPDATA) push(join(env.APPDATA, "npm", "node_modules", "jiti"));
  // node.exe 同目录的 `node_modules\jiti`
  if (env.NODE_EXE_DIR) push(join(env.NODE_EXE_DIR, "node_modules", "jiti"));
  void exists;
  return [...new Set(out)];
}

/**
 * 在多个 hash 目录中挑最新的一个（`resolveJiti` 用）：
 * 只接受含 `package.json` 且 `name === "jiti"` 的目录；按修改时间取最新（`mtimeMs` 缺失时退回字典序）。
 */
export function pickNewestJitiCandidate(dirs, deps = {}) {
  const readJson = deps.readJson ?? ((file) => {
    try {
      return JSON.parse(fs.readFileSync(file, "utf8"));
    } catch {
      return undefined;
    }
  });
  const mtime = deps.mtimeMs ?? ((file) => {
    try {
      return fs.statSync(file).mtimeMs;
    } catch {
      return 0;
    }
  });
  const valid = [];
  for (const dir of dirs) {
    const pkg = readJson(`${dir}/package.json`);
    if (pkg && pkg.name === "jiti") valid.push({ dir, mtime: mtime(`${dir}/package.json`) });
  }
  if (valid.length === 0) return undefined;
  valid.sort((a, b) => (b.mtime - a.mtime) || (a.dir < b.dir ? -1 : 1));
  return valid[0].dir;
}

/** 前序候选（不需要跑子进程就能得到的那些）。 */
export function staticJitiCandidates(extensionPaths, env = process.env) {
  const out = [];
  if (env.PI_JITI) out.push(env.PI_JITI);
  // Windows 布局（F34/W1）：`%LOCALAPPDATA%\pnpm\global\…` 等，Linux 上这些键不存在 ⇒ 空数组。
  out.push(...windowsJitiCandidates(env));
  for (const entry of extensionPaths) {
    const dir = path.dirname(entry);
    out.push(path.join(dir, "node_modules/jiti"));
    out.push(path.join(dir, "..", "node_modules/jiti"));
    out.push(path.join(dir, "..", "..", "node_modules/jiti"));
  }
  const packageDir = env.PI_PACKAGE_DIR
    ?? (env.TS_PI_ENTRY
      ? path.resolve(path.dirname(env.TS_PI_ENTRY), "../../..")
      : undefined);
  if (packageDir) out.push(path.join(packageDir, "node_modules/jiti"));
  return [...new Set(out)];
}

/** 命令输出 → 候选目录（纯函数；输出为空/多余空白/多行都能处理）。 */
export function commandJitiCandidates(stdout, deps = {}) {
  const root = String(stdout ?? "").trim().split(/\r?\n/)[0].trim();
  if (!root) return [];
  return [
    ...npmJitiCandidates(root),
    ...pnpmJitiCandidates(root, deps),
  ];
}

function resolveJitiDir(dir, exists = (file) => fs.existsSync(file)) {
  for (const candidate of [
    path.join(dir, "lib/jiti.cjs"),
    path.join(dir, "lib/jiti.mjs"),
    path.join(dir, "dist/jiti.mjs"),
  ]) {
    if (exists(candidate)) return { dir, entry: candidate };
  }
  return undefined;
}

/**
 * 找 jiti。`deps` 可注入（单测用）：
 * `{ runCommand(cmd) -> string, listDir, exists }`。
 * 命令只在静态候选全部落空时才执行；命令不存在/超时/空输出一律静默。
 */
export function resolveJiti(extensionPaths, deps = {}) {
  for (const dir of staticJitiCandidates(extensionPaths)) {
    deps.attempts?.push(dir);
    const found = resolveJitiDir(dir, deps.exists);
    if (found) return found;
  }
  const runCommand = deps.runCommand ?? ((cmd) => {
    try {
      const result = spawnSync(cmd, ["root", "-g"], {
        encoding: "utf-8",
        timeout: 3000,
        windowsHide: true,
      });
      return result.status === 0 ? result.stdout : "";
    } catch {
      return "";
    }
  });
  const commandDeps = { listDir: deps.listDir, exists: deps.exists };
  for (const cmd of ["npm", "pnpm"]) {
    for (const dir of commandJitiCandidates(runCommand(cmd), commandDeps)) {
      deps.attempts?.push(dir);
      const found = resolveJitiDir(dir, deps.exists);
      if (found) return found;
    }
  }
  return undefined;
}


/**
 * 扩展标签（F28/S2e）：与 pi-cli 的 `extension_labels()` 同规则——
 * 规范化路径（`<dir>/./index.ts` 里的 `.` 不算目录名）、去掉末尾 `index.*`、取所在目录名。
 * stderr 汇总与 `[Extension issues]` 都用它，避免出现 "."。
 */
export function extensionLabel(file) {
  const normalized = path.resolve(file);
  const base = path.basename(normalized);
  if (/^index\.(ts|js|mjs|mts)$/i.test(base)) {
    return path.basename(path.dirname(normalized));
  }
  return base;
}
