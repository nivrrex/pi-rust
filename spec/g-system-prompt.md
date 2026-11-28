# spec/g-system-prompt.md — 系统提示（卡 g）

**系统提示不从源码重写，而是以黄金录制里实际发出的请求体为准**，
再用源码解释每一段的来源。段落装配的逐段来源见 `spec/system-prompt-sections.md`（原先的 `spec/system-prompt.template.txt` 是 golden 挖空快照，已删除），
由 `scripts/extract-system-prompt.mjs` 从 `golden/<场景号>/requests.jsonl` 抽出。
**校验走 Rust 测试**（非循环）：`cargo test -p pi-core --test t3_12_system_prompt` 里的
`sections_assemble_into_the_request_system_content`——输入取自**会话文件**的 `sections`、
输出比对**请求体**的 system 正文，两条不同来源的记录互相印证。

> 原先的 `scripts/verify-system-prompt.mjs` 已删除：它把 `{{PROJECT_CONTEXT}}` 的内容
> **从同一条 golden 里抠出来再回填**，然后断言等于该 golden——这是循环验证，通过了也证明不了什么。


## 1. 线上形态

- 实际发出的 system message 是 `content` 为空字符串、`sections` 承载全部提示、`toolsAdded` 承载工具声明的结构化对象（见 `spec/a-types.md` 的 `SystemMessage`）。〔packages/coding-agent/src/core/system-prompt.ts:182-192〕
- `buildSystemPromptState` 明确：除非 `forceSystemPrompt` 被指定（那条路把提示放进 `content`、不带 sections），否则 `content` 恒为空串，提示全部在 sections 里。〔packages/coding-agent/src/core/system-prompt.ts:186-192〕
- section 的命名规则：`preamble` 是不带标签的裸文本，其余每个 section 都被同名标签包起来（`<name>\n…\n</name>`），目的是让模型能把后续更新对应回原 section。〔packages/coding-agent/src/core/system-prompt.ts:45-50〕〔packages/coding-agent/src/core/system-prompt.ts:175-179〕
- `buildSystemPrompt` 只是把 state 交给 `getSystemMessageText` 渲染，语义上与 transcript 里重放的结果一致。〔packages/coding-agent/src/core/system-prompt.ts:194-197〕

## 2. 黄金运行里出现的 section

以下是 `golden/S01/requests.jsonl` 里实际出现的 section，顺序即对象键顺序：

- `preamble`：没有自定义提示时的固定开场白 `You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.`；给了 `customPrompt` 就用后者。〔packages/coding-agent/src/core/system-prompt.ts:143-147〕
- `tools`：当前启用工具的 snippet 列表，末尾固定追加 `In addition to the tools above, you may have access to other custom tools depending on the project.`。〔packages/coding-agent/src/core/system-prompt.ts:151-151〕
- `rules`：由 `buildRules(selectedTools, toolGuidelines, promptGuidelines)` 生成，即每个选中工具的 guidelines 加上全局 `promptGuidelines`。〔packages/coding-agent/src/core/system-prompt.ts:152-152〕〔packages/coding-agent/src/core/system-prompt.ts:108-114〕
- `docs`：以 `Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):` 开头的固定模板，指向安装目录下的 README/docs/examples，**随安装位置变化**。〔packages/coding-agent/src/core/system-prompt.ts:153-153〕
- `cwd`：section 内容就是 cwd 本身（再由 section 包装逻辑加上 `<cwd>` 标签），并把反斜杠换成正斜杠，**随运行位置变化**。〔packages/coding-agent/src/core/system-prompt.ts:170-170〕
- 命中 `AGENTS.md` / `CLAUDE.md` 时会追加项目指令段落（已用 S23 录制验证，见 `golden/S23/requests.jsonl`）：整体包在 `Project-specific instructions and guidelines:` 这行之后，内层每个文件各占一个 `<project_instructions path="…">` 标签，文件按**全局目录在前、cwd 在后**排列。〔packages/coding-agent/src/core/system-prompt.ts:72-79〕

## 3. 变量部分

模板里的占位符只有三个，其余全部逐字保留：

| 占位符 | 含义 | 取值来源 |
|---|---|---|
| `{{CWD}}` | 会话工作目录 | 运行器传入的 cwd，源码里还会把 `\` 换成 `/`。〔packages/coding-agent/src/core/system-prompt.ts:170-170〕 |
| `{{PI_PACKAGE_DIR}}` | npm 包安装目录 | 由 `PI_PACKAGE_DIR` 或实际安装路径决定，出现在 docs 段落里 |
| `{{TOOLS_JSON}}` | 本次会话启用的工具声明数组 | `SystemMessage.toolsAdded`，内容是 `spec/e-tools.md` 里四个工具的 schema |
| `{{PROJECT_CONTEXT}}` | 项目指令段落 | cwd 的 `AGENTS.md` 与全局配置目录的 `CLAUDE.md`；无命中时该占位符不出现 |

`selectedTools` 的默认值是 `["read", "bash", "edit", "write"]`，与黄金里 tools 段的顺序一致。〔packages/coding-agent/src/core/system-prompt.ts:57-59〕

## 4. 校验方式

```bash
node scripts/extract-system-prompt.mjs S23                    # 用 S23（带项目指令）生成模板
cargo test -p pi-core --test t3_12_system_prompt              # 非循环校验：会话 sections → 请求体正文（31/32 场景）
node scripts/extract-system-prompt.mjs S01 /tmp/tpl-s01.txt   # 无项目指令时的模板
```

最近一次结果：`[verify] S23: …逐字节一致（2984 字节）`、`[verify] S01: …逐字节一致（2680 字节）`。

## 未覆盖 / 不确定


- `skills` section 的赋值条件与内容：`if (skillFileReadTool && skills.length > 0) { … }`，内容取自 `formatSkillsForPrompt(skills, skillFileReadTool)`（为空则不设）。〔packages/coding-agent/src/core/system-prompt.ts:166-169〕
- 赋值顺序（按源码写法）：`preamble` → `tools` → `rules` → `docs` → `addendum`（即 `appendSystemPrompt`）→ `project_context` → `skills` → `cwd`，之后才是调用方传入的自定义 `sections`——**skills 在 project_context 之后、cwd 之前**。（prompt template 的展开不在本文件，本卡未抽。）〔packages/coding-agent/src/core/system-prompt.ts:146-172〕
- `forceSystemPrompt` 在构建时**直接短路**：`if (input.forceSystemPrompt !== undefined) return { content: input.forceSystemPrompt };`——只给 `content`，**没有** `sections`。〔packages/coding-agent/src/core/system-prompt.ts:190-190〕
- 与之相对，`customPrompt` 不短路而是写进 `promptSections.preamble`。〔packages/coding-agent/src/core/system-prompt.ts:143-144〕
- `appendSystemPrompt` 在归一化时是**字符串**（缺省 `""`），与 `customPrompt`/`forceSystemPrompt` 同属提示文本组，**不是** `sections` 的成员。〔packages/coding-agent/src/core/system-prompt.ts:54-70〕
- section 名的合法性校验常量是 `SYSTEM_PROMPT_SECTION_NAME`，在本文件内定义。〔packages/coding-agent/src/core/system-prompt.ts:52-52〕
- 校验处对 `preamble` **额外豁免**：`!SYSTEM_PROMPT_SECTION_NAME.test(name) || name === "preamble"` 时走另一条分支。〔packages/coding-agent/src/core/system-prompt.ts:137-146〕
- 最终结构以 `preamble` 起头（**不带标签**，即未标记文本），其余每个 section 渲染成 `<名字>\n内容\n</名字>`。〔packages/coding-agent/src/core/system-prompt.ts:175-177〕
- 已验证：`skills` 与 `project_context` 并存时的先后顺序为 **`project_context` 在前、`skills` 在后**。判据是专门为此录制的场景 `golden/S23b`（全局目录放 skill、cwd 放 AGENTS.md，两者同时进入系统提示），连跑 3 次稳定；请求体里 `project_context` 段起始于第 2590 字节、`skills` 段起始于第 2775 字节。〔golden/S23b/requests.jsonl:1〕
- 完整的 section 顺序（已验证）：`preamble` → `tools` → `rules` → `docs` → `project_context` → `skills` → `cwd`；缺席的段整段跳过，段间以空行分隔。〔golden/S23b/session.jsonl:1〕
