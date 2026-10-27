# 思考档位

## 档位集合
推理模型：`off / minimal / low / medium / high`；
`xhigh`、`max` 仅当模型显式声明了对应 `thinkingLevelMap` 时才出现（上游 `models.ts:924-931`）。

## 交互
- `shift+tab` 在可用档位间循环（非推理模型提示不支持）；
- 选择器里 Enter 立即生效、`ctrl+s` 设为默认（写 `settings.json` 的 `default_thinking_level`）；
- footer 右侧在推理模型下显示 `model • <level>`，`off` 显示 `model • thinking off`。

## 复现
`scenarios-tui/T77-thinking-levels.json`（`extraModels: [{id, reasoning: true}]` 声明推理能力）。
