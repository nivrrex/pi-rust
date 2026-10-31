//! `Message` 联合，对应 `ref/pi-0.87.1/packages/ai/src/types.ts` 的
//! `Message`、`SystemMessage`、`UserMessage`、`AssistantMessage`、`ToolResultMessage`
//! （见 `spec/a-types.md` 第 8–15 节）。
//!
//! 互通要求：
//! - **未知字段一律保留**（`#[serde(flatten)] extra`），读后写回不丢信息；
//! - `ToolResult` 是**独立的消息变体**，不是内容块——session 与 compact 的
//!   cut point 逻辑按 role 分派，把它建模成块会让那部分很别扭（D6）。
//!
//! **为什么在 pi-ai 而不在 pi-core**（RUNBOOK-4 的 P3.0b）：provider 要消费 `Context`
//! 里的消息，而 pi-core 依赖 pi-ai；放在 pi-core 会让 pi-ai 反过来依赖 pi-core 而成环。

use crate::AssistantMessage;
pub use crate::Content;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 工具结果里的内容块。TS 侧是 `(TextContent | ImageContent)[]`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolResultContent {
    /// 文本块。
    Text {
        text: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// 图片块。
    ///
    /// **会话里存的是 `data` + `mimeType`**（golden/S25-anthropic 实测），
    /// 而请求体里的 `source` 形态属于 pi-ai 的 `ContextBlock`——两者不是同一个东西，
    /// 别混用。
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
}

/// 用户或系统消息的内容：TS 允许裸字符串或块数组。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    /// 裸字符串形态（golden 里 system 消息就是这种）。
    Text(String),
    /// 块数组形态。
    Blocks(Vec<Content>),
}

/// 一条消息。四个成员的联合，按 `role` 分派。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum Message {
    /// 系统消息。`content` 是字符串或文本块数组。
    System {
        content: UserContent,
        timestamp: i64,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// 用户消息。
    User {
        content: UserContent,
        timestamp: i64,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// 助手消息——直接复用 pi-ai 的 `AssistantMessage` 形态。
    Assistant(Box<AssistantMessage>),
    /// 工具结果消息。**独立变体**，不是内容块。
    ToolResult {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        content: Vec<ToolResultContent>,
        #[serde(rename = "isError")]
        is_error: bool,
        timestamp: i64,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// 扩展注入的自定义消息（ref `core/messages.ts:46-54` 的 `CustomMessage`）。
    ///
    /// agent 层角色；provider 边界由 [`to_llm_messages`]（ref `messages.ts:148-188` 的 `convertToLlm`）
    /// 折成 **user** 消息。
    Custom {
        #[serde(rename = "customType")]
        custom_type: String,
        content: UserContent,
        #[serde(default)]
        display: bool,
        #[serde(default)]
        details: Value,
        timestamp: i64,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// 分支摘要（ref `:56-61` 的 `BranchSummaryMessage`）。
    BranchSummary {
        summary: String,
        #[serde(rename = "fromId", default)]
        from_id: Option<String>,
        timestamp: i64,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// 压缩摘要（ref `:63-67` 的 `CompactionSummaryMessage`）。
    CompactionSummary {
        summary: String,
        #[serde(rename = "tokensBefore", default)]
        tokens_before: u64,
        timestamp: i64,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
}

/// `extra` 里存"原样写出的 `toolsAdded` 文本"的内部键。
///
/// 为什么要有它：`serde_json` 的 `Map` 是 `BTreeMap`，序列化会把 `parameters` 的键排序；
/// 而 TS 的 `declarationsEqual`（`transcript.ts:140-141`）用 `JSON.stringify` 比对工具声明，
/// **键序参与判等**。把按 TS 键序排好的数组文本放在这个键下，
/// 手写的 `Serialize` 就能用 `RawValue` **原样写出**（见 `impl Serialize for Message`）。
pub const TOOLS_ADDED_RAW_KEY: &str = "__toolsAddedRaw";

/// `extra` 里存"原样写出的 `sections` 文本"的内部键。
///
/// 与 [`TOOLS_ADDED_RAW_KEY`] 同理：TS 重放会话时会**按 `sections` 的文件顺序**拼系统提示
/// （`transcript.ts` 的 replay 逐段拼接），而 `serde_json::Map` 会把它排序成字母序
/// （`cwd, docs, preamble, rules, tools`），于是重放出来的提示与 TS 自己写的不同 ✗。
pub const SECTIONS_RAW_KEY: &str = "__sectionsRaw";

/// 其余三个变体的镜像：**只为了复用派生 Serialize**，字段与顺序与 `Message` 完全一致。
#[derive(Serialize)]
#[serde(tag = "role", rename_all = "camelCase")]
enum MessageMirror<'a> {
    User {
        content: &'a UserContent,
        timestamp: i64,
        #[serde(flatten)]
        extra: &'a Map<String, Value>,
    },
    Assistant(&'a AssistantMessage),
    ToolResult {
        #[serde(rename = "toolCallId")]
        tool_call_id: &'a str,
        #[serde(rename = "toolName")]
        tool_name: &'a str,
        content: &'a Vec<ToolResultContent>,
        #[serde(rename = "isError")]
        is_error: bool,
        timestamp: i64,
        #[serde(flatten)]
        extra: &'a Map<String, Value>,
    },
    Custom {
        #[serde(rename = "customType")]
        custom_type: &'a str,
        content: &'a UserContent,
        display: bool,
        details: &'a Value,
        timestamp: i64,
        #[serde(flatten)]
        extra: &'a Map<String, Value>,
    },
    BranchSummary {
        summary: &'a str,
        #[serde(rename = "fromId")]
        from_id: &'a Option<String>,
        timestamp: i64,
        #[serde(flatten)]
        extra: &'a Map<String, Value>,
    },
    CompactionSummary {
        summary: &'a str,
        #[serde(rename = "tokensBefore")]
        tokens_before: u64,
        timestamp: i64,
        #[serde(flatten)]
        extra: &'a Map<String, Value>,
    },
}

/// 手写 `Serialize`：**只有 `System` 变体特殊**（要把 `toolsAdded` 原样写出），其余委托镜像。
///
/// `Deserialize` 仍是派生的（`flatten` 保留 → 未知字段照旧收进 `extra`）。
impl Serialize for Message {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        match self {
            Message::System {
                content,
                timestamp,
                extra,
            } => {
                // 与原派生实现相同的顺序：`role`（内部标签）→ `content` → `timestamp` → `extra` 各键。
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("role", "system")?;
                map.serialize_entry("content", content)?;
                map.serialize_entry("timestamp", timestamp)?;
                for (key, value) in extra {
                    if key == SECTIONS_RAW_KEY {
                        // 原样写出（`RawValue` 不做任何重排）。
                        let raw = value.as_str().unwrap_or("{}");
                        let raw: &serde_json::value::RawValue =
                            serde_json::from_str(raw).map_err(serde::ser::Error::custom)?;
                        map.serialize_entry("sections", raw)?;
                        continue;
                    }
                    if key == TOOLS_ADDED_RAW_KEY {
                        // 原样写出（`RawValue` 不做任何重排）。
                        let raw = value.as_str().unwrap_or("[]");
                        let raw: &serde_json::value::RawValue =
                            serde_json::from_str(raw).map_err(serde::ser::Error::custom)?;
                        map.serialize_entry("toolsAdded", raw)?;
                        continue;
                    }
                    if key == "toolsAdded" && extra.contains_key(TOOLS_ADDED_RAW_KEY) {
                        // 有原样文本时，跳过被排序过的那份。
                        continue;
                    }
                    if key == "sections" && extra.contains_key(SECTIONS_RAW_KEY) {
                        // 同上：有原样文本时跳过被排序的那份。
                        continue;
                    }
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
            Message::User {
                content,
                timestamp,
                extra,
            } => MessageMirror::User {
                content,
                timestamp: *timestamp,
                extra,
            }
            .serialize(serializer),
            Message::Assistant(assistant) => {
                MessageMirror::Assistant(assistant).serialize(serializer)
            }
            Message::ToolResult {
                tool_call_id,
                tool_name,
                content,
                is_error,
                timestamp,
                extra,
            } => MessageMirror::ToolResult {
                tool_call_id,
                tool_name,
                content,
                is_error: *is_error,
                timestamp: *timestamp,
                extra,
            }
            .serialize(serializer),
            Message::Custom {
                custom_type,
                content,
                display,
                details,
                timestamp,
                extra,
            } => MessageMirror::Custom {
                custom_type,
                content,
                display: *display,
                details,
                timestamp: *timestamp,
                extra,
            }
            .serialize(serializer),
            Message::BranchSummary {
                summary,
                from_id,
                timestamp,
                extra,
            } => MessageMirror::BranchSummary {
                summary,
                from_id,
                timestamp: *timestamp,
                extra,
            }
            .serialize(serializer),
            Message::CompactionSummary {
                summary,
                tokens_before,
                timestamp,
                extra,
            } => MessageMirror::CompactionSummary {
                summary,
                tokens_before: *tokens_before,
                timestamp: *timestamp,
                extra,
            }
            .serialize(serializer),
        }
    }
}

/// 三个前缀/后缀常量（ref `core/messages.ts:11-24`，逐字复制）。
pub const COMPACTION_SUMMARY_PREFIX: &str =
    "The conversation history before this point was compacted into the following summary:\n\n<summary>\n";
/// 压缩摘要的收尾（ref `:16-17`）。
pub const COMPACTION_SUMMARY_SUFFIX: &str = "\n</summary>";
/// 分支摘要的开头（ref `:19-22`）。
pub const BRANCH_SUMMARY_PREFIX: &str =
    "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n";
/// 分支摘要的收尾（ref `:24`）。
pub const BRANCH_SUMMARY_SUFFIX: &str = "</summary>";

/// ref `core/messages.ts:148-188` `convertToLlm`：把三个 **agent 层**角色折成 user 消息，其余原样。
///
/// 在 pi-core 构造 `ProviderRequest.context` 之前调用一次即可；provider 层因此只见到 4 个已知 role。
#[must_use]
pub fn to_llm_messages(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .map(|message| match message {
            Message::Custom {
                content,
                timestamp,
                extra,
                ..
            } => Message::User {
                content: content.clone(),
                timestamp: *timestamp,
                extra: extra.clone(),
            },
            Message::BranchSummary {
                summary,
                timestamp,
                extra,
                ..
            } => Message::User {
                content: UserContent::Text(format!(
                    "{BRANCH_SUMMARY_PREFIX}{summary}{BRANCH_SUMMARY_SUFFIX}"
                )),
                timestamp: *timestamp,
                extra: extra.clone(),
            },
            Message::CompactionSummary {
                summary,
                timestamp,
                extra,
                ..
            } => Message::User {
                content: UserContent::Text(format!(
                    "{COMPACTION_SUMMARY_PREFIX}{summary}{COMPACTION_SUMMARY_SUFFIX}"
                )),
                timestamp: *timestamp,
                extra: extra.clone(),
            },
            _not_agent_role => message.clone(),
        })
        .collect()
}

impl Message {
    /// 消息的 role 字符串，与线上协议一致。
    pub fn role(&self) -> &'static str {
        match self {
            Self::System { .. } => "system",
            Self::User { .. } => "user",
            Self::Assistant(_) => "assistant",
            Self::ToolResult { .. } => "toolResult",
            Self::Custom { .. } => "custom",
            Self::BranchSummary { .. } => "branchSummary",
            Self::CompactionSummary { .. } => "compactionSummary",
        }
    }

    /// 时间戳（Unix 毫秒）。
    pub fn timestamp(&self) -> i64 {
        match self {
            Self::System { timestamp, .. }
            | Self::User { timestamp, .. }
            | Self::ToolResult { timestamp, .. }
            | Self::Custom { timestamp, .. }
            | Self::BranchSummary { timestamp, .. }
            | Self::CompactionSummary { timestamp, .. } => *timestamp,
            Self::Assistant(message) => message.timestamp,
        }
    }

    /// 内容块视图。system/user 的字符串形态会转成一个文本块；
    /// assistant 直接用其内容；toolResult 的块形状不同，返回空切片。
    pub fn blocks(&self) -> Vec<Content> {
        match self {
            Self::System { content, .. }
            | Self::User { content, .. }
            | Self::Custom { content, .. } => match content {
                UserContent::Text(text) => vec![Content::Text {
                    text: text.clone(),
                    text_signature: None,
                }],
                UserContent::Blocks(blocks) => blocks.clone(),
            },
            Self::Assistant(message) => message.content.clone(),
            Self::ToolResult { .. } => Vec::new(),
            Self::BranchSummary { summary, .. } | Self::CompactionSummary { summary, .. } => {
                vec![Content::Text {
                    text: summary.clone(),
                    text_signature: None,
                }]
            }
        }
    }
}

/// 消息级的转换规则（`transform-messages.ts:166-232`），**作用于 `Message` 本身**。
///
/// - assistant 的 `stopReason` 为 `error`/`aborted` → 整条丢弃（回放半截的 reasoning 或
///   未闭合的工具调用会让 provider 报错）；
/// - 未被应答的 tool call，在遇到下一条 user 消息或对话结束时，合成
///   `is_error: true` + `"No result provided"` 的 `ToolResult` 消息。
///
/// 返回**有序**的最终序列：合成结果排在触发它的 user 消息**之前**。
pub fn transform_messages(messages: &[Message]) -> Vec<Message> {
    fn tool_call_ids(message: &Message) -> Vec<(String, String)> {
        let Message::Assistant(assistant) = message else {
            return Vec::new();
        };
        assistant
            .content
            .iter()
            .filter_map(|block| match block {
                Content::ToolCall { id, name, .. } => Some((id.clone(), name.clone())),
                Content::Text { .. } | Content::Thinking { .. } | Content::Image { .. } => None,
            })
            .collect()
    }

    /// 该消息应答掉的 tool call id 集合。
    fn answered_ids(message: &Message) -> Vec<String> {
        match message {
            Message::ToolResult { tool_call_id, .. } => vec![tool_call_id.clone()],
            Message::System { .. }
            | Message::User { .. }
            | Message::Assistant(_)
            | Message::Custom { .. }
            | Message::BranchSummary { .. }
            | Message::CompactionSummary { .. } => Vec::new(),
        }
    }

    fn synthetic(id: &str, name: &str, timestamp: i64) -> Message {
        Message::ToolResult {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            content: vec![ToolResultContent::Text {
                text: "No result provided".to_string(),
                extra: serde_json::Map::new(),
            }],
            is_error: true,
            timestamp,
            extra: serde_json::Map::new(),
        }
    }

    let mut out: Vec<Message> = Vec::new();
    let mut pending: Vec<(String, String)> = Vec::new();

    for message in messages {
        let dropped = matches!(
            message,
            Message::Assistant(assistant)
                if matches!(assistant.stop_reason, crate::StopReason::Error | crate::StopReason::Aborted)
        );
        if dropped {
            // 丢弃前先补齐它之前遗留的待定 tool call。
            for (id, name) in pending.drain(..) {
                out.push(synthetic(&id, &name, message.timestamp()));
            }
            continue;
        }

        match message {
            Message::Assistant(_) => {
                let calls = tool_call_ids(message);
                if !calls.is_empty() {
                    for (id, name) in pending.drain(..) {
                        out.push(synthetic(&id, &name, message.timestamp()));
                    }
                    pending = calls;
                }
                out.push(message.clone());
            }
            Message::User { .. } => {
                // 该消息里的 toolResult 先应答掉对应的待定调用（`Message::User` 里不含
                // toolResult，这条留给 `ToolResult` 消息），剩下的才合成补齐。
                let answered = answered_ids(message);
                pending.retain(|(id, _)| !answered.contains(id));
                for (id, name) in pending.drain(..) {
                    out.push(synthetic(&id, &name, message.timestamp()));
                }
                out.push(message.clone());
            }
            Message::ToolResult { tool_call_id, .. } => {
                pending.retain(|(id, _)| id != tool_call_id);
                out.push(message.clone());
            }
            Message::System { .. }
            | Message::Custom { .. }
            | Message::BranchSummary { .. }
            | Message::CompactionSummary { .. } => out.push(message.clone()),
        }
    }
    for (id, name) in pending.drain(..) {
        out.push(synthetic(&id, &name, 0));
    }
    out
}
