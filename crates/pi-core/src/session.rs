//! session 文件的 entry 类型，对应 `packages/coding-agent/src/core/session-manager.ts`
//! （见 `spec/c-session.md` 第 1–2 节）。
//!
//! 互通要求：
//! - **未知的 entry 类型必须保留原样**（[`SessionEntry::Unknown`]），否则读到
//!   新版本写出的会话就会失败，或读后写回丢信息；
//! - 每条 entry 的公共字段是 `type` / `id` / `parentId` / `timestamp`。

use pi_ai::message::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 当前写出的 session 版本（`session-manager.ts` 的 `CURRENT_SESSION_VERSION`）。
pub const CURRENT_SESSION_VERSION: u32 = 3;

/// 文件首行的 header。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionHeader {
    /// 固定 `"session"`。
    #[serde(rename = "type")]
    pub kind: String,
    /// v1 的会话没有该字段，所以是可选的。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// 会话 id。
    pub id: String,
    /// ISO 时间戳。
    pub timestamp: String,
    /// 会话创建时的工作目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// 未识别字段原样保留。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 一条 session entry。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionEntry {
    /// `message`：带一条 `AgentMessage`。
    #[serde(rename = "message")]
    Message {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        message: Message,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// `thinking_level_change`。
    #[serde(rename = "thinking_level_change")]
    ThinkingLevelChange {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "thinkingLevel")]
        thinking_level: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// `model_change`。
    #[serde(rename = "model_change")]
    ModelChange {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        provider: String,
        #[serde(rename = "modelId")]
        model_id: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// `compaction`。
    #[serde(rename = "compaction")]
    Compaction {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        summary: String,
        #[serde(rename = "firstKeptEntryId")]
        first_kept_entry_id: String,
        #[serde(rename = "tokensBefore")]
        tokens_before: u64,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// `context_edit`：对某条 entry 的上下文做替换。
    #[serde(rename = "context_edit")]
    ContextEdit {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "targetId")]
        target_id: String,
        /// 允许显式 `null`（表示"移除该条"）。
        #[serde(default)]
        replacement: Value,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// `custom_message`：扩展写入的自定义消息。
    ///
    /// `convertToLlm`（`packages/coding-agent/src/core/messages.ts:163-169`）把它变成一条
    /// **user** 消息（`content` 是字符串时包成单个 text 块）——即**参与上下文**。
    #[serde(rename = "custom_message")]
    CustomMessage {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "customType")]
        custom_type: String,
        content: Value,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// `branch_summary`：分支摘要。
    ///
    /// `convertToLlm`（`packages/coding-agent/src/core/messages.ts:170-176`）把它变成一条
    /// **user** 消息：`BRANCH_SUMMARY_PREFIX` + `summary` + `BRANCH_SUMMARY_SUFFIX`。
    #[serde(rename = "branch_summary")]
    BranchSummary {
        id: String,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: String,
        #[serde(rename = "fromId")]
        from_id: String,
        summary: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    /// 其它 entry 类型（含未来新增的）——**原样保留**。
    #[serde(untagged)]
    Unknown(Value),
}

impl SessionEntry {
    /// entry 类型字符串。
    pub fn kind(&self) -> &str {
        match self {
            Self::Message { .. } => "message",
            Self::ThinkingLevelChange { .. } => "thinking_level_change",
            Self::ModelChange { .. } => "model_change",
            Self::Compaction { .. } => "compaction",
            Self::ContextEdit { .. } => "context_edit",
            Self::CustomMessage { .. } => "custom_message",
            Self::BranchSummary { .. } => "branch_summary",
            Self::Unknown(value) => value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("<无 type>"),
        }
    }

    /// entry id；未知类型也能取到（没有则为 `None`）。
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Message { id, .. }
            | Self::ThinkingLevelChange { id, .. }
            | Self::ModelChange { id, .. }
            | Self::Compaction { id, .. }
            | Self::ContextEdit { id, .. }
            | Self::CustomMessage { id, .. }
            | Self::BranchSummary { id, .. } => Some(id),
            Self::Unknown(value) => value.get("id").and_then(Value::as_str),
        }
    }

    /// 父 entry id。
    pub fn parent_id(&self) -> Option<&str> {
        match self {
            Self::Message { parent_id, .. }
            | Self::ThinkingLevelChange { parent_id, .. }
            | Self::ModelChange { parent_id, .. }
            | Self::Compaction { parent_id, .. }
            | Self::ContextEdit { parent_id, .. }
            | Self::CustomMessage { parent_id, .. }
            | Self::BranchSummary { parent_id, .. } => parent_id.as_deref(),
            Self::Unknown(value) => value.get("parentId").and_then(Value::as_str),
        }
    }
}

/// 文件首行：要么是 header，要么（旧文件）直接是 entry。
#[derive(Debug, Clone, PartialEq)]
pub enum SessionLine {
    /// header。
    Header(SessionHeader),
    /// 一条 entry。
    Entry(SessionEntry),
}

/// 解析会话文件的一行。
pub fn parse_line(line: &str) -> Result<SessionLine, serde_json::Error> {
    let value: Value = serde_json::from_str(line)?;
    if value.get("type").and_then(Value::as_str) == Some("session") {
        return Ok(SessionLine::Header(serde_json::from_value(value)?));
    }
    Ok(SessionLine::Entry(serde_json::from_value(value)?))
}
