//! 测试共用的构造助手。
//!
//! P3.0 收尾把 `ContextMessage` / `ContextBlock` / `Role` 从**产品 API** 删掉了
//! （`Context` 现在直接持有 `Message`）。但测试想表达的是"请求体形状的消息序列"，
//! 用 `ContextBlock` 那种表格化写法最省事，所以在**测试侧**保留一套同名的镜像类型，
//! 由本模块转成 `Message`。产品代码不再有这两个类型。

#![allow(dead_code)]

use pi_ai::{AssistantMessage, Content, Context, Message, StopReason, ToolResultContent, Usage};
use serde_json::{Map, Value};

/// 镜像旧的 `pi_ai::context::Role`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

/// 镜像旧的 `pi_ai::context::ContextBlock`。
#[derive(Debug, Clone)]
pub enum Block {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: Option<String>,
    },
    Image {
        media_type: String,
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: Vec<Block>,
        is_error: bool,
    },
}

/// 镜像旧的 `pi_ai::context::ContextMessage`。
#[derive(Debug, Clone)]
pub struct ContextMessage {
    pub role: Role,
    pub content: Vec<Block>,
    pub stop_reason: Option<StopReason>,
}

fn block_to_content(block: &Block) -> Option<Content> {
    match block {
        Block::Text { text } => Some(Content::Text {
            text: text.clone(),
            text_signature: None,
        }),
        Block::Thinking {
            thinking,
            signature,
        } => Some(Content::Thinking {
            thinking: thinking.clone(),
            thinking_signature: signature.clone(),
            redacted: None,
        }),
        Block::Image { media_type, data } => Some(Content::Image {
            data: data.clone(),
            mime_type: media_type.clone(),
            extra: Map::new(),
        }),
        Block::ToolUse { id, name, input } => Some(Content::ToolCall {
            id: id.clone(),
            name: name.clone(),
            arguments: match input {
                Value::Object(map) => map.clone(),
                Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::String(_)
                | Value::Array(_) => Map::new(),
            },
            thought_signature: None,
        }),
        // `tool_result` 不是 `Content` 的成员，单独成一条 `Message::ToolResult`。
        Block::ToolResult { .. } => None,
    }
}

fn tool_result_content(block: &Block) -> ToolResultContent {
    match block {
        Block::Text { text } => ToolResultContent::Text {
            text: text.clone(),
            extra: Map::new(),
        },
        Block::Image { media_type, data } => ToolResultContent::Image {
            data: data.clone(),
            mime_type: media_type.clone(),
            extra: Map::new(),
        },
        Block::Thinking { .. } | Block::ToolUse { .. } | Block::ToolResult { .. } => {
            ToolResultContent::Text {
                text: String::new(),
                extra: Map::new(),
            }
        }
    }
}

/// 把镜像消息序列转成 `Message`。
fn to_messages(messages: &[ContextMessage]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    for message in messages {
        match message.role {
            Role::System => out.push(Message::System {
                content: pi_ai::UserContent::Blocks(
                    message
                        .content
                        .iter()
                        .filter_map(block_to_content)
                        .collect(),
                ),
                timestamp: 0,
                extra: Map::new(),
            }),
            Role::Assistant => out.push(Message::Assistant(Box::new(AssistantMessage {
                role: "assistant".to_string(),
                content: message
                    .content
                    .iter()
                    .filter_map(block_to_content)
                    .collect(),
                api: "openai-completions".to_string(),
                provider: "mock".to_string(),
                model: "mock-model".to_string(),
                response_model: None,
                usage: Usage::default(),
                stop_reason: message.stop_reason.unwrap_or(StopReason::Stop),
                error_message: None,
                raw_stop_reason: None,
                end_turn: None,
                response_id: None,
                timestamp: 0,
            }))),
            Role::User => {
                // 非 tool_result 的块合成一条 user 消息；tool_result 各自成一条独立消息
                // （会话里它本就是独立消息，顺序按出现位置保留）。
                let plain: Vec<Content> = message
                    .content
                    .iter()
                    .filter(|block| !matches!(block, Block::ToolResult { .. }))
                    .filter_map(block_to_content)
                    .collect();
                if !plain.is_empty() {
                    out.push(Message::User {
                        content: pi_ai::UserContent::Blocks(plain),
                        timestamp: 0,
                        extra: Map::new(),
                    });
                }
                for block in &message.content {
                    if let Block::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } = block
                    {
                        out.push(Message::ToolResult {
                            tool_call_id: tool_use_id.clone(),
                            tool_name: String::new(),
                            content: content.iter().map(tool_result_content).collect(),
                            is_error: *is_error,
                            timestamp: 0,
                            extra: Map::new(),
                        });
                    }
                }
            }
        }
    }
    out
}

/// 镜像旧的 `Context::from_context_messages`。
pub fn ctx_msgs(system: Option<String>, messages: Vec<ContextMessage>) -> Context {
    Context::from_messages(system, &to_messages(&messages))
}

/// 简写：只有一条 user 文本消息。
pub fn ctx_user(prompt: &str, system: Option<&str>) -> Context {
    Context::from_messages(
        system.map(str::to_string),
        &[Message::User {
            content: pi_ai::UserContent::Text(prompt.to_string()),
            timestamp: 0,
            extra: Map::new(),
        }],
    )
}
