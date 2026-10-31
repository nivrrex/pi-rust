//! steering / follow-up 待处理队列。〔`agent.ts:142-172`、`agent.ts:244-245,296-324`〕
//!
//! 两种取出模式：`all` 一次全取、`one-at-a-time` 只取队首（默认）。
//! 取出条数由纯函数 [`super::drain_count`] 决定，本模块只负责线程安全地存与取。
//!
//! **并发**：外部（rpc 的 stdin 读取）往里 `push`、循环在自己的 turn 边界 `drain`，
//! 因此用 `Mutex` 保护；锁只在存取期间持有，不做任何 I/O，不会形成长临界区。

use pi_ai::message::{Message, UserContent};
use pi_ai::Content;
use std::sync::Mutex;

use super::{drain_count, DrainMode};

/// 一个待处理消息队列。
#[derive(Debug, Default)]
pub struct PendingQueue {
    messages: Mutex<Vec<Message>>,
}

impl PendingQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 入队。
    pub fn push(&self, message: Message) {
        if let Ok(mut guard) = self.messages.lock() {
            guard.push(message);
        }
    }

    /// 当前排队条数。
    pub fn len(&self) -> usize {
        self.messages
            .lock()
            .map(|guard| guard.len())
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 按模式取出若干条（从队首）。
    /// 队列里各条消息的**文本**（`queue_update` 事件的载荷要用它）。
    pub fn texts(&self) -> Vec<String> {
        self.messages
            .lock()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|message| match message {
                        Message::User {
                            content: UserContent::Text(text),
                            ..
                        } => Some(text.clone()),
                        Message::User {
                            content: UserContent::Blocks(blocks),
                            ..
                        } => Some(
                            blocks
                                .iter()
                                .filter_map(|block| match block {
                                    Content::Text { text, .. } => Some(text.clone()),
                                    Content::Image { .. }
                                    | Content::Thinking { .. }
                                    | Content::ToolCall { .. } => None,
                                })
                                .collect::<Vec<String>>()
                                .join(""),
                        ),
                        Message::Assistant(_)
                        | Message::System { .. }
                        | Message::ToolResult { .. }
                        | Message::Custom { .. }
                        | Message::BranchSummary { .. }
                        | Message::CompactionSummary { .. } => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn drain(&self, mode: DrainMode) -> Vec<Message> {
        let Ok(mut guard) = self.messages.lock() else {
            return Vec::new();
        };
        let take = drain_count(mode, guard.len());
        guard.drain(..take).collect()
    }

    /// 取出全部（`clear_queue` 语义）。
    pub fn drain_all(&self) -> Vec<Message> {
        self.drain(DrainMode::All)
    }
}

/// 把一段文本包成 `Message::User`（队列里存的一律是用户消息）。
pub fn user_message(text: &str) -> Message {
    Message::User {
        content: pi_ai::UserContent::Text(text.to_string()),
        timestamp: 0,
        extra: serde_json::Map::new(),
    }
}
