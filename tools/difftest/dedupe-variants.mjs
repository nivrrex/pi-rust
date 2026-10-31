#!/usr/bin/env node
// 把 collect-variants.sh 收集到的多次运行按 A 层去重，写出 variants/vN/ 并更新 meta.json。
import fs from "node:fs/promises";
import path from "node:path";

const [id, work, runsArg] = process.argv.slice(2);
const runs = Number(runsArg ?? 10);
const golden = path.join("golden", id);
const variantsDir = path.join(golden, "variants");
await fs.rm(variantsDir, { recursive: true, force: true });

const forms = new Map();
const perRun = [];
for (let i = 1; i <= runs; i++) {
  const dir = path.join(work, `r${i}`, id);
  let session = "", requests = "", events = "", fsJson = "[]", exitCode = null;
  try {
    session = await fs.readFile(path.join(dir, "session.jsonl"), "utf-8");
    requests = await fs.readFile(path.join(dir, "requests.jsonl"), "utf-8");
    events = await fs.readFile(path.join(dir, "events.jsonl"), "utf-8");
    fsJson = await fs.readFile(path.join(dir, "fs.json"), "utf-8");
    exitCode = JSON.parse(await fs.readFile(path.join(dir, "meta.json"), "utf-8")).exitCode;
  } catch {
    perRun.push({ run: i, ok: false });
    continue;
  }
  // A 层 = 会话语义内容 + 文件系统副作用 + 退出码
  const key = JSON.stringify([session, fsJson, exitCode]);
  const form = forms.get(key) ?? { n: forms.size + 1, count: 0, session, requests, events, fsJson, exitCode };
  form.count += 1;
  forms.set(key, form);
  perRun.push({ run: i, ok: true, form: form.n });
}

const list = [...forms.values()];
for (const form of list) {
  const dir = path.join(variantsDir, `v${form.n}`);
  await fs.mkdir(dir, { recursive: true });
  await fs.writeFile(path.join(dir, "session.jsonl"), form.session);
  await fs.writeFile(path.join(dir, "requests.jsonl"), form.requests);
  await fs.writeFile(path.join(dir, "events.jsonl"), form.events);
  await fs.writeFile(path.join(dir, "fs.json"), form.fsJson);
  await fs.writeFile(
    path.join(dir, "meta.json"),
    `${JSON.stringify({ form: form.n, count: form.count, exitCode: form.exitCode }, null, 2)}\n`,
  );
}

const metaPath = path.join(golden, "meta.json");
const meta = JSON.parse(await fs.readFile(metaPath, "utf-8"));
meta.variants = { runs, forms: list.length, counts: list.map((f) => ({ form: f.n, count: f.count })) };
await fs.writeFile(metaPath, `${JSON.stringify(meta, null, 2)}\n`);
console.log(
  `[variants] ${id}: ${runs} 次运行 → ${list.length} 种 A 层形态（${meta.variants.counts
    .map((c) => `v${c.form}×${c.count}`)
    .join(", ")}）`,
);
