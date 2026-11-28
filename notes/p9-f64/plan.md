# F64：思考档位映射表（`thinkingLevelMap`）

## 缺陷
`LIMITATIONS.md` “思考档位映射表”一节记录了真实缺口：上游按模型声明的
`thinkingLevelMap` 增删 `xhigh`/`max`，pi-rs 只按 `reasoning` 给固定七档。
后果：声明 `xhigh`/`max` 的模型少给最高两档；声明某档为 `null` 的模型多给该档。

## ref 规格（已核实）
- `packages/ai/src/models.ts:921-935` `getSupportedThinkingLevels`：
  - `!model.reasoning` ⇒ `["off"]`；
  - 否则按 `EXTENDED_THINKING_LEVELS` 过滤：`map[level] === null` ⇒ 排除；
    `level` 为 `xhigh`/`max` 时**仅当 `map[level] !== undefined`** 才保留；其余恒保留。
- `packages/ai/src/models.ts:937-955` `clampThinkingLevel`：不在集合内则先向后、
  再向前找最近可用档，都没有则 `availableLevels[0] ?? "off"`。
- 字段声明：`core/model-config.ts:194/209`（schema）、`extensions/types.ts:1696`。

## 现状
- `ModelDefinition`（`config.rs:255`）无该字段，值只落在 `extra` 里、无人读取。
- `ResolvedModel`（`config.rs:316`）同样没有。
- `thinking_selector::clamp_to_supported`（`:298`）只做 `EXTENDED_THINKING_LEVELS.contains`。

## 步骤
- [x] 1. `ModelDefinition.thinking_level_map: Option<BTreeMap<String, Option<String>>>`
      （`null` 与"缺失"必须可区分 ⇒ 值类型是 `Option<String>`）。
- [x] 2. `ResolvedModel.thinking_level_map` 同上，两个构造点填充。
- [x] 3. `thinking_selector::supported_levels(reasoning, map) -> Vec<&'static str>`
      照 `models.ts:921-935`。
- [x] 4. `clamp_to_supported_against(available, requested)` 照 `:937-955`；保留
      `clamp_to_supported` 为"无映射表"的旧行为薄封装（现有 9 个调用点不变）。
- [x] 5. 用例：`xhigh`/`max` 仅在声明时保留；`null` 档被排除；clamp 前后找最近档。

## 推翻条件
- 若 pi-rs 的九个调用点拿不到 `ResolvedModel`（只有 `reasoning: bool`），
  则 map 无法参与解析 ⇒ 需先扩签名；届时本卡超出 300 行，拆卡重排。

## 结果
5 步全部完成，`9` 个 `resolve_thinking_level` 调用点全部改走 `_with_map`。
`pi-ai` + `pi-cli` 全量 143 组 ok、0 失败。一条既有用例
（`per_model_beats_settings_default` 断言无映射表时 `xhigh` 存活）与 ref 相反，已修正。
遗留：内置目录不带该字段（见 LIMITATIONS）。

## F64 扩展（事件派发，10 项全部完成）
提交：`2a90895`(session_compact/failed)、`a377703`(session_info_changed、thinking_level_select)、
`8cc6532`(model_select)、`6ba7ecd`(ui_prompt_start/end)、`34b846a`(context_with_system、
before_provider_headers)、`eba5db9`(after_provider_response)、`9d79246`/`0c916ea`(文档与旧断言)。

- `SharedSink`（TUI+rpc）此前**没有** `before_provider_request` ⇒ 交互/rpc 下该事件从未派发；
  已补。此为顺带修复，属代用户扩大范围。
- `after_provider_response` 需扩接口：`Stream` 加 `response_meta`（与 `final_message` 同型），
  两个 HTTP provider 在 `bytes_stream()` 消费 `response` 之前填。
- 两处**待验证**：`thinking_level_select.previousLevel` 取自 writer 记录而非内存态；
  `session_compact.willRetry` 取 `reason=="overflow"` 而非"是否真重跑该回合"。
- 批末 L3：test-gate 285 组/1447 项、precommit 16/16、real-parity 0 格。
