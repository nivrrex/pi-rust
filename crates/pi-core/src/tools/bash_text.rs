//! bash 工具的**纯逻辑**部分：超时校验、尾部截断、输出与退出码文案。
//!
//! 逐条对应 `packages/coding-agent/src/core/tools/bash.ts:22-33、320-372`
//! 与 `spec/e-tools.md` 第 5 节。进程管理（spawn / killProcessTree / abort 监听）
//! 属边界层，不在这里。

use super::read_text::{TruncatedBy, DEFAULT_MAX_BYTES};

/// 超时上限（`bash.ts:22`，等于 i32 的毫秒上限）。
pub const MAX_TIMEOUT_MS: u64 = 2_147_483_647;

/// 超时上限的秒数（`bash.ts:23`，由毫秒上限除以 1000）。
pub const MAX_TIMEOUT_SECONDS: u64 = MAX_TIMEOUT_MS / 1000;

/// 超时参数的校验错误，文案逐字照 TS。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeoutError {
    /// 非有限数或 <= 0。
    #[error("Invalid timeout: must be a finite number of seconds")]
    NotFiniteOrNonPositive,
    /// 超过上限。
    #[error("Invalid timeout: maximum is {MAX_TIMEOUT_SECONDS} seconds")]
    TooLarge,
}

/// 把秒数解析成毫秒；`None` 表示不超时（TS 里没有默认超时）。
pub fn resolve_timeout_ms(seconds: Option<f64>) -> Result<Option<u64>, TimeoutError> {
    let Some(seconds) = seconds else {
        return Ok(None);
    };
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(TimeoutError::NotFiniteOrNonPositive);
    }
    let ms = seconds * 1000.0;
    if ms > MAX_TIMEOUT_MS as f64 {
        return Err(TimeoutError::TooLarge);
    }
    Ok(Some(ms as u64))
}

/// 尾部截断的结果（保留最后若干行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailTruncation {
    /// 保留下来的内容。
    pub content: String,
    /// 是否截断过。
    pub truncated: bool,
    /// 按什么截断。
    pub truncated_by: Option<TruncatedBy>,
    /// 原文总行数。
    pub total_lines: usize,
    /// 保留的行数。
    pub output_lines: usize,
    /// 保留的字节数。
    pub output_bytes: usize,
    /// 末行被字节上限切掉一半。
    pub last_line_partial: bool,
    /// 末行的原始字节数（用于部分截断时的提示）。
    pub last_line_bytes: usize,
}

/// 从尾部保留不超过 `max_lines` 行的内容（`truncate.ts` 的 `truncateTail`）。
///
/// 与头部截断相反：这里保留**末尾**，用于看命令最后输出了什么。
pub fn truncate_tail(content: &str, max_lines: usize, max_bytes: usize) -> TailTruncation {
    // 空内容算**零行**（oracle 实测：`truncateTail("")` 的 totalLines 是 0）——
    // `"".split('\n')` 会给出一个空串元素，直接当成一行是错的。
    let mut lines: Vec<&str> = if content.is_empty() {
        Vec::new()
    } else {
        content.split('\n').collect()
    };
    if content.ends_with('\n') {
        lines.pop();
    }
    let total_lines = lines.len();
    let total_bytes = content.len();
    if total_lines <= max_lines && total_bytes <= max_bytes {
        return TailTruncation {
            content: content.to_string(),
            truncated: false,
            truncated_by: None,
            total_lines,
            output_lines: total_lines,
            output_bytes: total_bytes,
            last_line_partial: false,
            last_line_bytes: lines.last().map_or(0, |line| line.len()),
        };
    }

    // 从尾部往前收，够行数或够字节就停。
    let mut kept: Vec<&str> = Vec::new();
    let mut bytes = 0_usize;
    let mut truncated_by = TruncatedBy::Lines;
    for line in lines.iter().rev() {
        if kept.len() >= max_lines {
            truncated_by = TruncatedBy::Lines;
            break;
        }
        let line_bytes = line.len() + usize::from(!kept.is_empty());
        if bytes + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            break;
        }
        kept.push(line);
        bytes += line_bytes;
    }
    kept.reverse();

    // 一行都放不下时，`truncateTail` 会**返回部分行**（oracle 实测：单行超限时
    // content 是该行末尾 maxBytes 字节，并置 lastLinePartial）——这是尾部截断与
    // 头部截断的关键差别：头部从不返回半行，尾部可以。
    let last_line_bytes = lines.last().map_or(0, |line| line.len());
    // 只在**行数上限允许**、而是字节上限挡住时走部分行回退：
    // `maxLines: 0` 是行数约束，此时应当没有任何内容（oracle 实测）。
    if kept.is_empty() && !lines.is_empty() && max_lines > 0 {
        let last = lines.last().copied().unwrap_or_default();
        let raw = last.as_bytes();
        let mut start = raw.len().saturating_sub(max_bytes);
        // 切点落在字符中间时**向前对齐到下一个字符边界**，而不是产生替换符：
        // oracle 实测 `"中"×20` 配 maxBytes=20 得到 6 个中（18 字节），多出的 2 字节被丢掉。
        while start < raw.len() && !last.is_char_boundary(start) {
            start += 1;
        }
        let partial = last.get(start..).unwrap_or_default().to_string();
        let partial_bytes = partial.len();
        return TailTruncation {
            content: partial,
            truncated: true,
            truncated_by: Some(TruncatedBy::Bytes),
            total_lines,
            output_lines: 1,
            output_bytes: partial_bytes,
            last_line_partial: true,
            last_line_bytes,
        };
    }

    TailTruncation {
        content: kept.join("\n"),
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        output_lines: kept.len(),
        output_bytes: bytes,
        last_line_partial: false,
        last_line_bytes,
    }
}

/// 人类可读的大小，格式与 TS 的 `formatSize` 一致。
///
/// 由 oracle 实测（`tools/oracle/truncate-oracle.mjs`）：`999 => "999B"`、
/// `1024 => "1.0KB"`、`1536 => "1.5KB"`、`51200 => "50.0KB"`——KB 保留**一位小数**，
/// 不是取整。我原先写成取整，会得到 `0KB` 这种明显不对的值。
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        return format!("{bytes}B");
    }
    format!("{:.1}KB", bytes as f64 / 1024.0)
}

/// 组装 bash 工具返回的文本（`bash.ts:320-336`）。
///
/// `full_output_path` 是完整输出落盘的路径；截断提示里会带上它。
pub fn format_output(
    content: &str,
    truncation: &TailTruncation,
    full_output_path: Option<&str>,
    empty_text: &str,
) -> String {
    let mut text = if content.is_empty() {
        empty_text.to_string()
    } else {
        content.to_string()
    };
    if !truncation.truncated {
        return text;
    }
    let start_line = truncation.total_lines - truncation.output_lines + 1;
    let end_line = truncation.total_lines;
    let path = full_output_path.unwrap_or_default();
    if truncation.last_line_partial {
        text.push_str(&format!(
            "\n\n[Showing last {} of line {end_line} (line is {}). Full output: {path}]",
            format_size(truncation.output_bytes),
            format_size(truncation.last_line_bytes)
        ));
    } else if truncation.truncated_by == Some(TruncatedBy::Lines) {
        text.push_str(&format!(
            "\n\n[Showing lines {start_line}-{end_line} of {}. Full output: {path}]",
            truncation.total_lines
        ));
    } else {
        text.push_str(&format!(
            "\n\n[Showing lines {start_line}-{end_line} of {} ({} limit). Full output: {path}]",
            truncation.total_lines,
            format_size(DEFAULT_MAX_BYTES)
        ));
    }
    text
}

/// 把状态行追加到输出后面（`bash.ts:341`）：有输出时空两行，没有就单独一行。
pub fn append_status(text: &str, status: &str) -> String {
    if text.is_empty() {
        status.to_string()
    } else {
        format!("{text}\n\n{status}")
    }
}

/// 退出码对应的错误文案；正常退出返回 `None`。
pub fn exit_status_message(exit_code: Option<i32>) -> Option<String> {
    match exit_code {
        Some(0) => None,
        Some(code) => Some(format!("Command exited with code {code}")),
        None => Some("Command terminated without an exit code".to_string()),
    }
}
