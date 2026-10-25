#!/usr/bin/env node
// tools/oracle/models-config-oracle.mjs —— 用 TS 版 coding-agent 的 ModelConfig 解析
// 多种 models.json 形态，产出 Rust 必须一致的期望结果。
//
// 必须在网络隔离内运行（见 tools/oracle/run-models-config.sh）。
// 期望值完全由 TS 产生，不手写。

import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const PACKAGE_INDEX =
  process.env.PI_CODING_AGENT_INDEX ??
  `${PKG}/index.js";
const OUT = path.resolve("fixtures/models-config");

const FORMS = {
  "provider-level-inherited": {
    description: "模型不写 api/baseUrl，继承 provider 级",
    providers: {
      p: { baseUrl: "https://a.example/v1", api: "openai-completions", models: [{ id: "m" }] },
    },
    probe: ["p", "m"],
  },
  "model-level-override": {
    description: "模型级 api 覆盖 provider 级",
    providers: {
      p: {
        baseUrl: "https://a.example/v1",
        api: "openai-completions",
        models: [{ id: "m", api: "anthropic-messages" }],
      },
    },
    probe: ["p", "m"],
  },
  "builtin-baseurl-only": {
    description: "内置 provider 只覆盖 baseUrl",
    providers: { openai: { baseUrl: "https://b.example/v1", models: [] } },
    probe: ["openai", "any"],
  },
  "headers-and-authheader": {
    description: "headers 与 authHeader",
    providers: {
      p: {
        baseUrl: "https://a.example/v1",
        api: "openai-completions",
        authHeader: false,
        headers: { "X-Trace": "1" },
        models: [{ id: "m" }],
      },
    },
    probe: ["p", "m"],
  },
  compat: {
    description: "compat 开关（含未知开关）",
    providers: {
      p: {
        baseUrl: "https://a.example/v1",
        api: "openai-completions",
        compat: { supportsStore: false, supportsUsageInStreaming: true, somethingUnknown: 1 },
        models: [{ id: "m" }],
      },
    },
    probe: ["p", "m"],
  },
  "cost-absent": {
    description: "cost 缺省",
    providers: { p: { baseUrl: "https://a.example/v1", api: "openai-completions", models: [{ id: "m" }] } },
    probe: ["p", "m"],
  },
  "cost-zero": {
    description: "cost 全零",
    providers: {
      p: {
        baseUrl: "https://a.example/v1",
        api: "openai-completions",
        models: [{ id: "m", cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }],
      },
    },
    probe: ["p", "m"],
  },
  "empty-models": {
    description: "空 models 数组",
    providers: { p: { baseUrl: "https://a.example/v1", api: "openai-completions", models: [] } },
    probe: ["p", "m"],
  },
  "models-absent": {
    description: "连 models 字段都没有",
    providers: { p: { baseUrl: "https://a.example/v1", api: "openai-completions" } },
    probe: ["p", "m"],
  },
  "missing-api": {
    description: "provider 与模型都没有 api",
    providers: { p: { baseUrl: "https://a.example/v1", models: [{ id: "m" }] } },
    probe: ["p", "m"],
  },
  "missing-baseurl": {
    description: "没有 baseUrl",
    providers: { p: { api: "openai-completions", models: [{ id: "m" }] } },
    probe: ["p", "m"],
  },
  "oauth-radius": {
    description: "oauth 凭据形态",
    providers: {
      p: { name: "P", baseUrl: "https://a.example/v1", api: "openai-completions", oauth: "radius", models: [{ id: "m" }] },
    },
    probe: ["p", "m"],
  },
};

await fs.mkdir(OUT, { recursive: true });
const tmp = await fs.mkdtemp("/tmp/models-config-oracle-");
// ModelConfig 不在公开 API，但 ModelRuntime.create({modelsPath}) 会走 ModelConfig.load，
// ModelRegistry 再包一层查询接口——两者都在包的公开导��里。
const mod = await import(pathToFileURL(PACKAGE_INDEX).href);
const { ModelRuntime, ModelRegistry } = mod;

const results = {};
for (const [name, form] of Object.entries(FORMS)) {
  const file = path.join(tmp, `${name}.json`);
  const modelsJson = { providers: form.providers };
  await fs.writeFile(file, `${JSON.stringify(modelsJson, null, 2)}\n`);
  // 形态源文件也留一份，Rust 侧测试直接读它，保证两侧跑的是同一份输入。
  await fs.writeFile(path.join(OUT, `${name}.json`), `${JSON.stringify(modelsJson, null, 2)}\n`);
  let entry;
  try {
    const runtime = await ModelRuntime.create({
      modelsPath: file,
      modelsStorePath: path.join(tmp, `${name}-store.json`),
    });
    const registry = new ModelRegistry(runtime);
    await registry.refresh();
    const [providerId, modelId] = form.probe;
    const found = registry.find(providerId, modelId);
    const configured = registry.getAll().filter((model) => model.provider === providerId);
    entry = {
      description: form.description,
      probe: form.probe,
      loadError: registry.getError() ?? null,
      found: found
        ? {
            provider: found.provider,
            id: found.id,
            api: found.api,
            baseUrl: found.baseUrl ?? null,
            maxTokens: found.maxTokens ?? null,
            contextWindow: found.contextWindow ?? null,
            cost: found.cost ?? null,
            reasoning: found.reasoning ?? null,
          }
        : null,
      // 该 provider 下 TS 实际认得的模型 id 列表（用于核对继承与覆盖）。
      modelIds: configured.map((model) => model.id).sort(),
    };
  } catch (error) {
    entry = {
      description: form.description,
      probe: form.probe,
      thrown: error instanceof Error ? error.message : String(error),
    };
  }
  results[name] = entry;
  await fs.writeFile(path.join(OUT, `${name}.expected.json`), `${JSON.stringify(entry, null, 2)}\n`);
  console.log(`[models-config] ${name}: loadError=${entry.loadError ?? "none"} found=${entry.found ? "yes" : "no"} ids=${JSON.stringify(entry.modelIds ?? [])}`);
}
await fs.writeFile(path.join(OUT, "index.json"), `${JSON.stringify(Object.keys(results), null, 2)}\n`);

// 额外导出一份内置目录：Rust 侧要实现"省略模型定义时回落到内置 catalog"，
// 就必须拿到 TS 实际认得的完整定义，而不是只有 id 列表。
try {
  const runtime = await ModelRuntime.create({ modelsPath: null });
  const registry = new ModelRegistry(runtime);
  await registry.refresh();
  const byProvider = {};
  for (const model of registry.getAll()) {
    (byProvider[model.provider] ??= []).push({
      id: model.id,
      name: model.name,
      api: model.api,
      baseUrl: model.baseUrl ?? null,
      maxTokens: model.maxTokens ?? null,
      contextWindow: model.contextWindow ?? null,
      cost: model.cost ?? null,
      reasoning: model.reasoning ?? null,
      input: model.input ?? null,
    });
  }
  for (const list of Object.values(byProvider)) list.sort((a, b) => a.id.localeCompare(b.id));
  await fs.writeFile(path.join(OUT, "builtin-catalog.json"), `${JSON.stringify(byProvider, null, 2)}\n`);
  console.log(`[models-config] 内置目录: ${Object.keys(byProvider).length} 个 provider，${Object.values(byProvider).reduce((n, l) => n + l.length, 0)} 个模型`);
} catch (error) {
  console.log(`[models-config] 内置目录导出失败: ${error instanceof Error ? error.message : String(error)}`);
}
