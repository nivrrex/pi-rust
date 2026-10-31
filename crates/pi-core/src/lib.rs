//! pi-core：agent loop、工具、session、compaction。
//!
//! 规格来源：`ref/pi-0.87.1/packages/agent/src/` 与 `packages/coding-agent/src/core/`，
//! 抽取结果见 `spec/b-agent-loop.md`、`spec/c-session.md`、`spec/d-compaction.md`、`spec/e-tools.md`。
//! P3 起填充：`message` 已落地，agent loop / 工具 / session / compaction 待做。

#![forbid(unsafe_code)]

pub mod agent;
pub mod project_context;
pub mod prompt_templates;
pub mod session;
pub mod session_io;
pub mod session_path;
pub mod skills;
pub mod system_prompt;
pub mod timefmt;
pub mod tools;

// `Message` 这类 provider 也要消费的类型现在住在 pi-ai（P3.0b 的分层要求），
// 这里只做转出口，避免下游改 import。
pub use pi_ai::message::{Message, ToolResultContent, UserContent};

/// crate 版本，与 `Cargo.toml` 保持一致。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `convertToLlm` 的薄转发（ref `core/messages.ts:148-188`）：摘要请求要先把 agent 层角色
/// 折成 LLM 消息再序列化（`compaction.ts:747` 的 `convertToLlm(currentMessages)`）。
#[must_use]
pub fn message_to_llm_for_summary(
    messages: &[pi_ai::message::Message],
) -> Vec<pi_ai::message::Message> {
    pi_ai::message::to_llm_messages(messages)
}
