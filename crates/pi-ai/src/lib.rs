//! pi-ai：消息类型、Provider 抽象、SSE 解析与重试。
//!
//! 规格来源：`ref/pi-0.87.1/packages/ai/src/`，抽取结果见 `spec/a-types.md`、`spec/b-agent-loop.md`。
//! G0（B 段）实现了 SSE 解析（V1）、tool call 增量拼装（V2）与取消（V3），
//! 判据是 `fixtures/expected/*.expected.json`。

#![forbid(unsafe_code)]

pub mod anthropic;
pub mod config;
pub mod context;
pub mod credentials;
pub mod decoder;
pub mod message;
pub mod openai;
pub mod overflow;
pub mod provider;
pub mod responses;
pub mod retry;
pub mod sse;
pub mod types;

pub use config::{ConfigError, ModelCost, ModelsConfig, ProviderConfig, ResolvedModel};
pub use context::Context;
pub use credentials::{
    resolve_credential, AuthSource, CredentialSources, EnvReader, ProcessEnv, ResolvedCredential,
};
pub use decoder::{
    calculate_context_tokens, calculate_cost, DecodeError, Rates, StreamDecoder,
    ANTHROPIC_MESSAGES, OPENAI_COMPLETIONS,
};
pub use message::{transform_messages, Message, ToolResultContent, UserContent};
pub use overflow::{is_context_overflow, is_recoverable_length};
pub use provider::{BoxFuture, Provider, ProviderError, ProviderRequest, Stream};
pub use retry::{Jitter, RetryDelayTooLong, RetryPolicy, RetrySleeper, DEFAULT_MAX_RETRY_DELAY_MS};
pub use sse::{SseError, SseEvent, SseParser};
pub use types::{AssistantEvent, AssistantMessage, Content, Cost, StopReason, Usage};

/// crate 版本，与 `Cargo.toml` 保持一致。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
