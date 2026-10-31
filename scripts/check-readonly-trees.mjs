#!/usr/bin/env node
// 只读目录守卫：ref/ 与 TS 安装包 **只读**。
//
// 背景（P8e-14/I）：P8e-11 做 jiti 实验时用符号链接指向真实 TS 安装包，随后写入
// 覆盖了它的 `package.json`。本脚本对关键文件做哈希校验，防止再次发生。
//
// 哈希清单在 `tools/guard/readonly-hashes.json`（**只含相对路径，不含用户目录**）。
// 路径解析：
//   - ref/ 用仓库内的 `ref/pi-0.87.1`；
//   - TS 包用 `PI_TS_PACKAGE_DIR`，或从 `TS_PI_ENTRY`（…/dist/bundle/cli.js）上推两级。
// 任一侧不存在 ⇒ 跳过该侧并打印说明（不是失败）。
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const repo = path.resolve(new URL("..", import.meta.url).pathname);
const manifestPath = path.join(repo, "tools/guard/readonly-hashes.json");
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));

const sha256 = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");

function resolveTsDir() {
  if (process.env.PI_TS_PACKAGE_DIR) return process.env.PI_TS_PACKAGE_DIR;
  if (process.env.TS_PI_ENTRY) return path.resolve(path.dirname(process.env.TS_PI_ENTRY), "../..");
  return undefined;
}

let failed = 0;
function check(label, base, entries) {
  if (!base || !fs.existsSync(base)) {
    console.log(`[readonly] ${label}: 未找到（跳过）`);
    return;
  }
  for (const [rel, want] of Object.entries(entries)) {
    if (!want) continue;
    const file = path.join(base, rel);
    if (!fs.existsSync(file)) {
      console.log(`[readonly] ${label}: 缺少 ${rel}`);
      failed += 1;
      continue;
    }
    const got = sha256(file);
    if (got !== want) {
      console.log(`[readonly] ${label}: **${rel} 已被修改**`);
      console.log(`          期望 ${want}`);
      console.log(`          实际 ${got}`);
      failed += 1;
    }
  }
  if (!failed) console.log(`[readonly] ${label}: ${Object.keys(entries).length} 个文件哈希一致`);
}

check("ref/pi-0.87.1", path.join(repo, "ref/pi-0.87.1"), manifest.ref);
check("TS 安装包", resolveTsDir(), manifest.tsPackage);

if (failed) {
  console.error("[readonly] 只读目录被改动：ref/ 与 TS 安装包只能读，实验请复制到临时目录再进行。");
  process.exit(1);
}
