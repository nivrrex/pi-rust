#!/usr/bin/env node
// 用 pi-ai 的 calculateCost 生成 cost 的逐位 oracle（R2）。
//
// 用法：PI_AI_PKG=<pi-ai 目录>/dist/index.js node tools/oracle/cost-oracle.mjs > fixtures/expected/cost-oracle.json
//
// 覆盖边界组合：token 数为 0/1/5/8/10/20 与非整除值，cacheWrite1h 有无，
// 以及带 tiers（按 inputTokensAbove 选档）的情形。

const { calculateCost } = await import(process.env.PI_AI_PKG);

const baseModel = (cost) => ({
  id: "m",
  name: "m",
  api: "openai-completions",
  provider: "mock",
  baseUrl: "",
  reasoning: false,
  input: ["text"],
  cost,
  contextWindow: 1,
  maxTokens: 1,
});

const cases = [];
const push = (label, cost, usage) => {
  const model = baseModel(cost);
  const u = {
    input: usage.input ?? 0,
    output: usage.output ?? 0,
    cacheRead: usage.cacheRead ?? 0,
    cacheWrite: usage.cacheWrite ?? 0,
    totalTokens: (usage.input ?? 0) + (usage.output ?? 0),
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
  };
  if (usage.cacheWrite1h !== undefined) u.cacheWrite1h = usage.cacheWrite1h;
  calculateCost(model, u);
  // 附带位模式：避免下游 JSON 解析器的十进制→f64 舍入差异（Q1）。
  const bits = Object.fromEntries(
    ["input", "output", "cacheRead", "cacheWrite", "total"].map((k) => [
      k,
      "0x" + Buffer.from(new Float64Array([u.cost[k]]).buffer).readBigUInt64LE().toString(16).padStart(16, "0"),
    ]),
  );
  cases.push({ label, cost, usage, result: u.cost, bits });
};

const rate = { input: 1, output: 2, cacheRead: 0.5, cacheWrite: 3 };
for (const n of [0, 1, 5, 7, 8, 10, 13, 20, 33, 100, 1000, 123457]) {
  push(`plain-${n}`, rate, { input: n, output: n });
}
push("cache-mix", rate, { input: 11, output: 3, cacheRead: 7, cacheWrite: 5 });
push("cache-write-1h", rate, { input: 4, output: 2, cacheRead: 1, cacheWrite: 6, cacheWrite1h: 4 });
push("cache-write-all-1h", rate, { input: 0, output: 0, cacheRead: 0, cacheWrite: 9, cacheWrite1h: 9 });
push("tier-hit", { ...rate, tiers: [{ ...rate, inputTokensAbove: 100 }] }, { input: 200, output: 10 });
push("tier-miss", { ...rate, tiers: [{ ...rate, inputTokensAbove: 1000 }] }, { input: 200, output: 10 });
push("tier-multi", {
  ...rate,
  tiers: [
    { ...rate, inputTokensAbove: 1000 },
    { ...rate, inputTokensAbove: 100, output: 4 },
  ],
}, { input: 500, output: 10 });
push("plain-1234567", rate, { input: 1234567, output: 7654321 });
push("frac-rate", { input: 0.123, output: 7.77, cacheRead: 0.001, cacheWrite: 12.5 }, { input: 37, output: 19 });

console.log(JSON.stringify(cases, null, 2));
