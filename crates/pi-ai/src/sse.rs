//! SSE 行解析器，对应 `spec/a-types.md` 第 4/8 节描述的事件协议所依赖的传输层。
//!
//! 只做传输层：把任意切分的字节流还原成一个个 SSE 事件，不理解任何 provider 的载荷语义。
//! 必须处理的边界（见 `fixtures/sse/README.md`）：
//! - CRLF 与 LF 两种行尾；
//! - 多字节 UTF-8 字符被切在两个网络块之间（切分只发生在 `\n` 之前，残余字节保留到下一次 push）；
//! - 一个事件由多条 `data:` 行组成；
//! - `:` 开头的注释行与 `event:`/`id:`/`retry:` 字段。

use thiserror::Error;

/// 一个已完成的 SSE 事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` 字段，没有则为 `None`。
    pub event: Option<String>,
    /// 所有 `data:` 行的内容，用 `\n` 连接。
    pub data: String,
    /// `id:` 字段，没有则为 `None`。
    pub id: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SseError {
    #[error("事件流已结束后又收到数据")]
    Closed,
}

/// 增量 SSE 解析器。跨 `push` 调用保留未成行的残余字节。
#[derive(Debug, Default)]
pub struct SseParser {
    /// 尚未遇到行尾的残余字节。
    pending: Vec<u8>,
    event: Option<String>,
    data: String,
    id: Option<String>,
    closed: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段字节，返回本次喂入完成的所有事件。
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, SseError> {
        if self.closed && !bytes.is_empty() {
            return Err(SseError::Closed);
        }
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut consumed = 0usize;
        while let Some(nl) = self.pending[consumed..].iter().position(|b| *b == b'\n') {
            let start = consumed;
            let end = consumed + nl;
            consumed = end + 1;
            let mut line = self.pending[start..end].to_vec();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if let Some(event) = self.push_line(&line)? {
                events.push(event);
            }
        }
        self.pending.drain(..consumed);
        Ok(events)
    }

    /// 流结束时调用；返回未以空行收尾的残余事件（如果有）。
    pub fn finish(&mut self) -> Result<Option<SseEvent>, SseError> {
        if self.closed {
            return Ok(None);
        }
        self.closed = true;
        if self.pending.is_empty() {
            return Ok(None);
        }
        let mut line = std::mem::take(&mut self.pending);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        self.push_line(&line)
    }

    /// 处理一行；返回 `Some` 表示这一行（空行）结束了一个事件。
    fn push_line(&mut self, line: &[u8]) -> Result<Option<SseEvent>, SseError> {
        if line.is_empty() {
            return Ok(self.take_event());
        }
        if line[0] == b':' {
            return Ok(None);
        }
        let Ok(text) = std::str::from_utf8(line) else {
            return Err(SseError::Closed);
        };
        let (field, value) = match text.find(':') {
            Some(idx) => {
                let value = &text[idx + 1..];
                (&text[..idx], value.strip_prefix(' ').unwrap_or(value))
            }
            None => (text, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            }
            "id" => self.id = Some(value.to_string()),
            // SSE 里还有 retry 等字段，本项目不关心。
            _ => {} // lint-banned: allow-wildcard
        }
        Ok(None)
    }

    fn take_event(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() && self.event.is_none() {
            return None;
        }
        let event = SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data),
            id: self.id.take(),
        };
        self.id = None;
        Some(event)
    }
}
