# g2. 系统提示的段落装配（来源：TS 源码）

本文件**取代**原先的 `spec/system-prompt.template.txt`（已删除）。

**为什么删**：那个文件是**从 golden/S23 挖空 `{{PROJECT_CONTEXT}}` 得到的单行快照**，
把它当规格会导致循环验证（"模板 + 从同一条 golden 抠出的段落 = golden"），
且它只有一个占位符，根本不是"带四个占位符的模板"——我此前按后者理解过，是错的。

段落装配的**唯一规格**是 TS 源码；下表逐段给出 `文件:行号`。

## 1. 装配入口

| 内容 | 位置 |
|---|---|
| 段落装配函数 | `packages/coding-agent/src/core/system-prompt.ts:121-180` |
| 归一化入参（含各字段默认值） | 同文件 `:54-70` |
| 返回"完整的提示状态"（`content` 为空串 + `sections`） | 同文件 `:186-192` |
| 拼成完整文本（`content` 优先，丢弃空段，`\n\n` 连接） | `packages/ai/src/utils/text.ts:15-21` |

## 2. 逐段来源与顺序

顺序取自 `system-prompt.ts:175-178`：先放 `preamble`（**不加包裹**），
其余段一律包成 `` `<${name}>\n${content}\n</${name}>` ``。

| 段名 | 内容来源 | 何时出现 |
|---|---|---|
| `preamble` | `customPrompt` 优先（`:143-145`）；否则默认文本 `:146-147` | 恒有 |
| `tools` | 可见工具列表 `- <name>: <snippet>` + 固定尾句，`:149-151` | 无 `customPrompt` 时 |
| `rules` | `buildRules(...)`（`:81-120`）：**首条规则按条件加**（`bash` 是否选中），其余去重后拼接 | 同上 |
| `docs` | 逐字文本 + 三个路径替换，`:153-160` | 同上 |
| `addendum` | `appendSystemPrompt`，`:163` | 非空时 |
| `project_context` | `renderProjectContext(contextFiles)`（`:72-78`）：每份文件包成 `<project_instructions path="...">\n{content}\n</project_instructions>`，再包一层 `<project_context>` | `contextFiles.length > 0` |
| `skills` | `formatSkillsForPrompt(skills, skillFileReadTool).trim()`，`:165-169`；`skillFileReadTool` 是 `read`/`bash` 中**先被选中**的那个 | 有该工具**且** `skills` 非空**且** trim 后非空 |
| `cwd` | `cwd.replace(/\\/g, "/")`（**只有反斜杠替换**），`:170` | 恒有 |
| 自定义段 | `normalizeBuildSystemPromptOptions` 的 `sections`，`:171-173`；名字须匹配 `^[a-z][a-z0-9_-]*$`（`:52`）且**不得叫 `preamble`**（`:136-140`） | 名字合法且内容非空 |

工具片段（`toolSnippets`）与工具指南（`toolGuidelines`）分别来自各工具定义文件，
例如 `read.ts:21-22`、`bash.ts:44-45`、`edit.ts:44,46-49`、`write.ts:17-18`。

## 3. 本仓库的实现状态

| 段 | 生产实现 | 值级验证 |
|---|---|---|
| `preamble` / `rules` / `tools` / `docs` / `cwd` | `crates/pi-core/src/system_prompt.rs`（`build_sections`，`:367`） | **非循环**：`rules_section_matches_golden_without_taking_golden_as_input`、`tools_section_ordering_comes_from_the_code_not_golden` |
| `project_context` | **未实现加载**（`main.rs:264-266` 传空）——发现规则属 P6 | 仅**格式**验证：`project_context_section_reproduces_the_golden_paragraph`（**输入值取自 golden**） |
| `skills` | **未实现加载**（同上）——属 P6 | 仅**格式**验证：`skills_section_reproduces_the_golden_paragraph`（**输入值取自 golden**） |

因此 **S23/S23b 的 C 层**在 P4 五层表里标"**不比较，原因：加载属 P6**"，不计入缺陷。
