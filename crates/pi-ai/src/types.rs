//! 消息与内容类型，对应 `ref/pi-0.87.1/packages/ai/src/types.ts`（见 `spec/a-types.md`）。
//!
//! G0 只需要 assistant 侧的类型：内容块、usage、停止原因、助手消息与流事件。
//! user / system / toolResult 消息在后续任务卡补齐。

use serde::{Deserialize, Serialize};

/// 停止原因，取值与 `StopReason` 一致（见 `spec/a-types.md` 第 4 节）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Pending,
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
    Deferred,
}

/// token 用量与费用，字段与 `Usage` 一致。
// **手写/旧版本会话可能缺字段**：TS 明确"parsed without validation"
// （`packages/coding-agent/src/core/session-manager.ts:442-443` 的注释），它**容忍**缺字段并把
// `content == null` 归一化。所以这里给数值/费用字段加 `serde(default)`——
// 否则一条缺 `cost` 的消息会让整个条目**解析失败**，我们就与 TS 分道扬镳了。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// `cacheWrite` 中按 1h 保留写入的子集；`calculateCost` 会对这部分按两倍 input 单价计。
    #[serde(rename = "cacheWrite1h", skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    pub total_tokens: u64,
    pub cost: Cost,
}

/// 费用明细，字段与 `Usage["cost"]` 一致。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

impl Default for Usage {
    fn default() -> Self {
        Self {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: None,
            reasoning: None,
            total_tokens: 0,
            cost: Cost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
                total: 0.0,
            },
        }
    }
}

/// 内容块，`type` 是判别字段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Content {
    Text {
        text: String,
        #[serde(rename = "textSignature", skip_serializing_if = "Option::is_none")]
        text_signature: Option<String>,
    },
    Thinking {
        thinking: String,
        #[serde(rename = "thinkingSignature", skip_serializing_if = "Option::is_none")]
        thinking_signature: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        redacted: Option<bool>,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Map<String, serde_json::Value>,
        #[serde(rename = "thoughtSignature", skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    /// 用户消息里的图片块。
    ///
    /// **TS 侧确实允许**：`UserMessage.content: string | (TextContent | ImageContent)[]`
    /// （`packages/ai/src/types.ts:509-513`），`ImageContent` 是 `{type:"image", data, mimeType}`
    /// （同文件 380-384）。我原先的 `Content` 只有 Text/Thinking/ToolCall，**缺这个变体**——
    /// 于是"顶层用户图片块"没法在 `Message` 里表示，这正是 P3.0 数据合并的拦路石。
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(flatten)]
        extra: serde_json::Map<String, serde_json::Value>,
    },
}

impl Content {
    /// 内容块的类型标签，与 JSON 里的 `type` 一致。
    pub fn kind(&self) -> &'static str {
        match self {
            Content::Text { .. } => "text",
            Content::Thinking { .. } => "thinking",
            Content::ToolCall { .. } => "toolCall",
            Content::Image { .. } => "image",
        }
    }
}

/// `AssistantMessage.role` 的反序列化缺省值——内部标签联合会消费掉该字段。
fn assistant_role() -> String {
    "assistant".to_string()
}

/// 助手消息。字段与 `AssistantMessage` 对齐，省略了 G0 用不到的诊断与延迟句柄字段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    /// 固定为 `"assistant"`。**反序列化时缺省**——因为 `pi_core::Message` 用
    /// `#[serde(tag = "role")]` 的内部标签联合包住它，标签会被枚举消费掉，
    /// 内部结构体就看不到该字段（serde 内部标签 + newtype 变体的已知限制）。
    // `skip_serializing`：**序列化时由 `Message` 的内部标签 `role` 输出**，
    // 这里再写一次就会得到重复键 `{"role":"assistant","role":"assistant",…}` ✗。
    // （S27 的对拍发现的保真缺陷：JSON 解析器取最后一个重复键，所以互操作与比较器都没暴露它。）
    #[serde(default = "assistant_role", skip_serializing)]
    pub role: String,
    pub content: Vec<Content>,
    pub api: String,
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    pub usage: Usage,
    pub stop_reason: StopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_turn: Option<bool>,
    /// provider 给的响应标识（TS `packages/ai/src/types.ts:522` 的 `responseId`；openai-completions
    /// 取流里首个 chunk 的 `id`，照 ref `packages/ai/src/api/openai-completions.ts:558`
    /// `output.responseId ||= chunk.id;`）。缺省不序列化 ⇒ 既有会话与判据不变。
    #[serde(rename = "responseId", skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    pub timestamp: i64,
}

/// `AssistantMessageEvent` 的成员（去掉 `partial` 快照，见 `spec/a-types.md` 第 8 节）。
///
/// `partial` 在 TS 版是共享的实时快照；Rust 侧不在线上协议里携带它，
/// 需要快照的调用方用 [`crate::decoder::Decoder::snapshot`] 取。
#[derive(Debug, Clone, PartialEq)]
pub enum AssistantEvent {
    Start,
    TextStart {
        content_index: usize,
    },
    TextDelta {
        content_index: usize,
        delta: String,
    },
    TextEnd {
        content_index: usize,
        content: String,
    },
    ThinkingStart {
        content_index: usize,
    },
    ThinkingDelta {
        content_index: usize,
        delta: String,
    },
    ThinkingEnd {
        content_index: usize,
        content: String,
    },
    ToolCallStart {
        content_index: usize,
    },
    ToolCallDelta {
        content_index: usize,
        delta: String,
    },
    ToolCallEnd {
        content_index: usize,
    },
    Done {
        reason: StopReason,
    },
    Error {
        reason: StopReason,
        /// provider 原文；与 TS 的 `errorMessage` 一致。
        message: String,
    },
}

impl AssistantEvent {
    /// 事件的 `type` 标签，与 JSON 线协议一致。
    pub fn type_name(&self) -> &'static str {
        match self {
            AssistantEvent::Start => "start",
            AssistantEvent::TextStart { .. } => "text_start",
            AssistantEvent::TextDelta { .. } => "text_delta",
            AssistantEvent::TextEnd { .. } => "text_end",
            AssistantEvent::ThinkingStart { .. } => "thinking_start",
            AssistantEvent::ThinkingDelta { .. } => "thinking_delta",
            AssistantEvent::ThinkingEnd { .. } => "thinking_end",
            AssistantEvent::ToolCallStart { .. } => "toolcall_start",
            AssistantEvent::ToolCallDelta { .. } => "toolcall_delta",
            AssistantEvent::ToolCallEnd { .. } => "toolcall_end",
            AssistantEvent::Done { .. } => "done",
            AssistantEvent::Error { .. } => "error",
        }
    }
}
