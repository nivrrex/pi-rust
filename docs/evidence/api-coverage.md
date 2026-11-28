# API 覆盖与用户配置盘点

## 已实现
- `anthropic-messages`
- `openai-completions`
- `openai-responses`（含工具调用往返、`incomplete` 终态）

## 未实现（用户配置未使用，仅登记）
`azure-openai-responses`、`openai-codex-responses`、`mistral-conversations`、
`bedrock-converse-stream`、`google-generative-ai`、`google-vertex`、`pi-messages`。

## 盘点结论
对当前用户的 `models.json` / `settings.json` 逐个模型取 `api` 字段（provider 级未声明时以模型级为准），
实际用到的 api 全部落在上面"已实现"三项内 ⇒ 不需要为这些配置新增 api 实现。
盘点明细（含 provider 名）保留在本地 `notes/evidence/X5-apis.md`，不进公开仓库。
