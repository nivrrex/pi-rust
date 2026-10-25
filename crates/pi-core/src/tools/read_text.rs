//! read 工具的**纯逻辑**部分：文本读取的切片、截断与提示文案。
//!
//! 逐条对应 `packages/coding-agent/src/core/tools/read.ts:140-183` 与
//! `tools/truncate.ts:70-150`（见 `spec/e-tools.md` 第 2 节）。
//! 这里不做任何 I/O——文件读取与 access 检查在边界层。

/// 头部截断的行数上限（`truncate.ts:11`）。
pub const DEFAULT_MAX_LINES: usize = 2000;

/// 头部截断的字节上限（`truncate.ts:12`，50KB）。
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024;

/// 文本读取的失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadTextError {
    /// `offset` 超出文件总行数（`read.ts:143` 的文案）。
    #[error("Offset {offset} is beyond end of file ({total_lines} lines total)")]
    OffsetBeyondEnd {
        /// 请求的 1 基行号。
        offset: usize,
        /// 文件总行数。
        total_lines: usize,
    },
}

/// 截断结果（`tools/truncate.ts:15-45` 的 `TruncationResult`）。
///
/// **字段名与键序照 golden 的 `details.truncation` 实测**（S21）：`content`、
/// `truncated`、`truncatedBy`、`totalLines`、`totalBytes`、`outputLines`、`outputBytes`、
/// `lastLinePartial`、`firstLineExceedsLimit`、`maxLines`、`maxBytes`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Truncation {
    /// 保留下来的内容。
    pub content: String,
    /// 是否发生了截断。
    pub truncated: bool,
    /// 按什么截断：`"lines"` 或 `"bytes"`；未截断时为 `None`（该键随之省略）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated_by: Option<TruncatedBy>,
    /// 原内容的总行数。
    pub total_lines: usize,
    /// 原内容的总字节数。
    pub total_bytes: usize,
    /// 输出的行数。
    pub output_lines: usize,
    /// 输出的字节数。
    pub output_bytes: usize,
    /// 最后一行是否被字节上限截断（只保留前缀）。
    pub last_line_partial: bool,
    /// 首行单独就超字节上限——此时不返回内容，让模型改用 bash。
    pub first_line_exceeds_limit: bool,
    /// 生效的行数上限。
    pub max_lines: usize,
    /// 生效的字节上限。
    pub max_bytes: usize,
}

/// 截断维度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TruncatedBy {
    /// 触到行数上限。
    Lines,
    /// 触到字节上限。
    Bytes,
}

/// 按行切分并计数；末尾换行不算多一行（`truncate.ts:47-55`）。
/// **截断**用的行切分：末尾的换行**不**产生一个空行。
///
/// 依据 oracle（`fixtures/truncate/cases.json` 的 `just-newlines` 形态）：TS 的
/// `truncateHead` 对只有换行的内容报 `outputLines` 与"去掉尾随空行"一致。
fn split_lines(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = content.split('\n').collect();
    if content.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// **read 选行**用的切分：保留末尾换行产生的空元素。
///
/// 这样 `lines[..].join("\n")` 会**还原**尾随换行——golden/S02 实测 TS 读到
/// `alpha\nbeta\n` 时工具结果就是 `alpha\nbeta\n`（我原先两处共用一个 pop 过的
/// 切分，结果把尾换行吃掉了；由 `t3_26_loop_scenarios` 的场景重放抓出）。
/// 与 [`split_lines`] 的差异**不是笔误**：TS 侧 read 的行选择与 truncateHead 的行计数
/// 本就用了不同的口径，这一点由 `t3_20_truncate_oracle` 的对账反证。
fn split_lines_for_read(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    content.split('\n').collect()
}

/// 从头部保留不超过 `max_lines` 行且不超过 `max_bytes` 字节的**完整行**。
///
/// 「不返回半行」是刻意的：`truncateHead` 的注释明确写了这一点。
pub fn truncate_head(content: &str, max_lines: usize, max_bytes: usize) -> Truncation {
    let lines = split_lines(content);
    let total_lines = lines.len();
    let total_bytes = content.len();

    if total_lines <= max_lines && total_bytes <= max_bytes {
        return Truncation {
            content: content.to_string(),
            truncated: false,
            truncated_by: None,
            total_lines,
            total_bytes,
            output_lines: total_lines,
            output_bytes: total_bytes,
            last_line_partial: false,
            first_line_exceeds_limit: false,
            max_lines,
            max_bytes,
        };
    }

    if lines.first().map_or(0, |line| line.len()) > max_bytes {
        return Truncation {
            content: String::new(),
            truncated: true,
            truncated_by: Some(TruncatedBy::Bytes),
            total_lines,
            total_bytes,
            output_lines: 0,
            output_bytes: 0,
            last_line_partial: false,
            first_line_exceeds_limit: true,
            max_lines,
            max_bytes,
        };
    }

    let mut kept: Vec<&str> = Vec::new();
    let mut bytes = 0_usize;
    let mut truncated_by = TruncatedBy::Lines;
    for (index, line) in lines.iter().enumerate().take(max_lines) {
        // 首行之外每行要算一个换行符。
        let line_bytes = line.len() + usize::from(index > 0);
        if bytes + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            break;
        }
        kept.push(line);
        bytes += line_bytes;
    }
    if kept.len() >= max_lines && bytes <= max_bytes {
        truncated_by = TruncatedBy::Lines;
    }

    Truncation {
        content: kept.join("\n"),
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        output_lines: kept.len(),
        output_bytes: bytes,
        // pi-rs 与 ref 一样**不返回半行**（`truncate.ts` 的注释），所以恒为 false；
        // golden/S21 实测也是 `lastLinePartial: false`。
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
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

/// 把文件全文渲染成 read 工具返回的文本。
///
/// 判定顺序与 TS 一致（`read.ts:159-183`）：
/// `firstLineExceedsLimit` → `truncation.truncated` → 用户 `limit` 提前截断 → 原样返回。
pub fn render_text_read(
    content: &str,
    offset: Option<usize>,
    limit: Option<usize>,
    max_lines: usize,
    max_bytes: usize,
) -> Result<(String, Option<Truncation>), ReadTextError> {
    let all_lines = split_lines_for_read(content);
    // `offset` 是 1 基，转 0 基；不给就是 0。
    // `usize` 不会为负 ⇒ 不需要 `.max(0)`（clippy 1.99 的 unnecessary_min_or_max）。
    let start_line = offset.map_or(0, |offset| offset.saturating_sub(1));
    if start_line >= all_lines.len() && !all_lines.is_empty() {
        return Err(ReadTextError::OffsetBeyondEnd {
            offset: offset.unwrap_or(1),
            total_lines: all_lines.len(),
        });
    }
    let start_line_display = start_line + 1;
    let total_file_lines = all_lines.len();

    let (selected, user_limited_lines) = match limit {
        Some(limit) => {
            let end = (start_line + limit).min(all_lines.len());
            (
                all_lines[start_line..end].join("\n"),
                Some(end - start_line),
            )
        }
        None => (all_lines[start_line..].join("\n"), None),
    };

    let truncation = truncate_head(&selected, max_lines, max_bytes);
    if truncation.first_line_exceeds_limit {
        let first = all_lines.get(start_line).copied().unwrap_or_default();
        return Ok((format!(
            "[Line {start_line_display} is {}, exceeds {} limit. Use bash: sed -n '{start_line_display}p' or read with offset/limit to view smaller ranges.]",
            format_size(first.len()),
            format_size(max_bytes)
        ), Some(truncation)));
    }
    if truncation.truncated {
        let end_line_display = start_line_display + truncation.output_lines.saturating_sub(1);
        let next_offset = end_line_display + 1;
        let mut text = truncation.content.clone();
        match truncation.truncated_by {
            Some(TruncatedBy::Lines) | None => text.push_str(&format!(
                "\n\n[Showing lines {start_line_display}-{end_line_display} of {total_file_lines}. Use offset={next_offset} to continue.]"
            )),
            Some(TruncatedBy::Bytes) => text.push_str(&format!(
                // 与 TS 一致：这里写的是 `formatSize(DEFAULT_MAX_BYTES)`，即**硬编码默认值**，
                // 不是本次实际生效的上限（`read.ts` 的怪癖，bash 侧同样如此）。
                "\n\n[Showing lines {start_line_display}-{end_line_display} of {total_file_lines} ({} limit). Use offset={next_offset} to continue.]",
                format_size(DEFAULT_MAX_BYTES)
            )),
        }
        return Ok((text, Some(truncation)));
    }
    if let Some(user_limited) = user_limited_lines {
        if start_line + user_limited < all_lines.len() {
            let remaining = all_lines.len() - (start_line + user_limited);
            let next_offset = start_line + user_limited + 1;
            return Ok((
                format!(
                    "{}\n\n[{remaining} more lines in file. Use offset={next_offset} to continue.]",
                    truncation.content
                ),
                None,
            ));
        }
    }
    Ok((truncation.content, None))
}
