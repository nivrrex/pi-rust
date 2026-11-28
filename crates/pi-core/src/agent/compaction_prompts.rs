//! compaction 的提示词常量（P5-2b）：**逐字**取自 ref，不做任何改写。
//!
//! ref：`packages/coding-agent/src/core/compaction/compaction.ts:529/562/599/964` 与 `utils.ts:156`。
//! 提取脚本处理 TS 模板字面量的转义，并断言"要么无插值、要么只插值已知常量"。

/// ref 常量 `SUMMARIZATION_PROMPT`（逐字）。
pub const SUMMARIZATION_PROMPT: &str = r####"The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.

Use this EXACT format:

## Goal
[What is the user trying to accomplish? Can be multiple items if the session covers different tasks.]

## Constraints & Preferences
- [Any constraints, preferences, or requirements mentioned by user]
- [Or "(none)" if none were mentioned]

## Progress
### Done
- [x] [Completed tasks/changes]

### In Progress
- [ ] [Current work]

### Blocked
- [Issues preventing progress, if any]

## Key Decisions
- **[Decision]**: [Brief rationale]

## Next Steps
1. [Ordered list of what should happen next]

## Critical Context
- [Any data, examples, or references needed to continue]
- [Or "(none)" if not applicable]

Keep each section concise. Preserve exact file paths, function names, and error messages."####;

/// ref 常量 `UPDATE_SUMMARIZATION_INSTRUCTIONS`（逐字）。
pub const UPDATE_SUMMARIZATION_INSTRUCTIONS: &str = r####"Update the existing structured summary with new information. RULES:
- PRESERVE all existing information from the previous summary
- ADD new progress, decisions, and context from the new messages
- UPDATE the Progress section: move items from "In Progress" to "Done" when completed
- UPDATE "Next Steps" based on what was accomplished
- PRESERVE exact file paths, function names, and error messages
- If something is no longer relevant, you may remove it

Use this EXACT format:

## Goal
[Preserve existing goals, add new ones if the task expanded]

## Constraints & Preferences
- [Preserve existing, add new ones discovered]

## Progress
### Done
- [x] [Include previously done items AND newly completed items]

### In Progress
- [ ] [Current work - update based on progress]

### Blocked
- [Current blockers - remove if resolved]

## Key Decisions
- **[Decision]**: [Brief rationale] (preserve all previous, add new)

## Next Steps
1. [Update based on current state]

## Critical Context
- [Preserve important context, add new if needed]

Keep each section concise. Preserve exact file paths, function names, and error messages."####;

/// ref 常量 `TURN_PREFIX_SUMMARIZATION_PROMPT`（逐字）。
pub const TURN_PREFIX_SUMMARIZATION_PROMPT: &str = r###"The messages above are earlier context from an ongoing conversation. Later messages are stored separately and do not need to be reconstructed.

Create a concise checkpoint of the user's request and the progress shown above. This checkpoint will be placed before the later messages so the conversation can continue with the necessary context.

## Original Request
[What did the user ask for?]

## Progress So Far
- [Key decisions and work completed in these messages]

## Context Needed to Continue
- [Information from these messages needed to understand the later work]

Only summarize information explicitly present above. Do not infer or recreate later messages."###;

/// ref 常量 `SUMMARIZATION_SYSTEM_PROMPT`（逐字，来自 `utils.ts:156`）。
pub const SUMMARIZATION_SYSTEM_PROMPT: &str = r#"You are a context summarization assistant. Your task is to read a conversation between a user and an AI assistant, then produce a structured summary following the exact format specified.

Do NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary."#;

/// ref `compaction.ts:599-601` 的 `UPDATE_SUMMARIZATION_PROMPT`：
/// `前缀 + UPDATE_SUMMARIZATION_INSTRUCTIONS`（ref 用模板插值拼出来，这里用函数等价实现）。
pub const UPDATE_SUMMARIZATION_PROMPT_PREFIX: &str = r#"The messages above are NEW conversation messages to incorporate into the existing summary provided in <previous-summary> tags.

"#;

/// 等价于 ref 的 `UPDATE_SUMMARIZATION_PROMPT`。
#[must_use]
pub fn update_summarization_prompt() -> String {
    format!("{UPDATE_SUMMARIZATION_PROMPT_PREFIX}{UPDATE_SUMMARIZATION_INSTRUCTIONS}")
}
