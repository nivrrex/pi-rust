//! 本实例的扩展日志内存缓冲（F44/S0）。
//!
//! 为什么需要：TUI 模式下宿主把扩展的 stdout/stderr 写进 `<agentDir>/extensions.log`，
//! 退出时再把"本次会话新增"的部分输出。用**文件偏移**算增量在并发多实例下会带出别的
//! 实例写进来的行（用户常同时开多个 pi-rs）。改为**每实例内存缓冲**：宿主每写一行就同时
//! 存进本缓冲，退出时只输出缓冲内容 ⇒ 天然按实例隔离。
//!
//! 容量：上限 [`DEFAULT_LIMIT_BYTES`]（256 KiB）。超出时丢**最旧**的行并记录丢弃行数，
//! 输出前提示"已丢弃 N 行"，避免静默截断。
//!
//! 线程安全：内部 `Mutex`，`push` 由宿主的单写入任务调用、`snapshot` 由退出路径调用；
//! 两者可能并发（退出时后台任务仍在跑），因此所有访问都在锁内完成。

use std::collections::VecDeque;

/// 默认缓冲上限（256 KiB）。
pub const DEFAULT_LIMIT_BYTES: usize = 256 * 1024;

/// 一个实例的日志缓冲。
#[derive(Debug)]
pub struct LogBuffer {
    lines: std::sync::Mutex<Inner>,
    limit: usize,
}

#[derive(Debug)]
struct Inner {
    lines: VecDeque<String>,
    bytes: usize,
    dropped: u64,
}

impl LogBuffer {
    /// 新建（默认上限）。
    #[must_use]
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_LIMIT_BYTES)
    }

    /// 指定上限新建（测试用）。
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            lines: std::sync::Mutex::new(Inner {
                lines: VecDeque::new(),
                bytes: 0,
                dropped: 0,
            }),
            limit: limit.max(1),
        }
    }

    /// 追加一行（含换行符）。
    pub fn push(&self, line: &str) {
        let Ok(mut inner) = self.lines.lock() else {
            return;
        };
        inner.bytes += line.len();
        inner.lines.push_back(line.to_string());
        while inner.bytes > self.limit && inner.lines.len() > 1 {
            if let Some(old) = inner.lines.pop_front() {
                inner.bytes = inner.bytes.saturating_sub(old.len());
                inner.dropped += 1;
            }
        }
    }

    /// 取出全部内容与"因超限丢弃的行数"（不清空缓冲）。
    #[must_use]
    pub fn snapshot(&self) -> (String, u64) {
        let Ok(inner) = self.lines.lock() else {
            return (String::new(), 0);
        };
        (inner.lines.iter().cloned().collect(), inner.dropped)
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::LogBuffer;

    #[test]
    fn keeps_order_and_reports_dropped_lines() {
        let buffer = LogBuffer::with_limit(10);
        buffer.push("aaaa\n");
        buffer.push("bbbb\n");
        buffer.push("cccc\n");
        let (text, dropped) = buffer.snapshot();
        assert!(dropped >= 1, "超限应丢最旧的行：{dropped}");
        assert!(text.ends_with("cccc\n"), "应保留最新的行：{text:?}");
        assert!(!text.contains("aaaa"), "最旧的行应被丢掉：{text:?}");
    }

    #[test]
    fn no_drop_below_limit() {
        let buffer = LogBuffer::new();
        buffer.push("one\n");
        buffer.push("two\n");
        let (text, dropped) = buffer.snapshot();
        assert_eq!(dropped, 0);
        assert_eq!(text, "one\ntwo\n");
    }
}
