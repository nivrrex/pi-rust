//! compaction 的**纯逻辑**（P5-1，对应 ref `core/compaction/compaction.ts` 的 token/cut-point 部分）。
//!
//! 本文件只放纯函数，不做任何 I/O；期望值来自 TS oracle
//! （`tools/oracle/compaction-oracle.mjs` → `fixtures/oracle/compaction-p5-1.json`），
//! 测试**只做逐字段一致比较**，不自己写期望（RUNBOOK-6 P5.1d）。
use crate::session::SessionEntry;
use pi_ai::message::{Message, UserContent};
use pi_ai::Usage;

/// 触发阈值设置（ref `compaction.ts:142-152`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionSettings {
    pub enabled: bool,
    pub reserve_tokens: u64,
    pub keep_recent_tokens: u64,
}

/// ref `compaction.ts:148-152`：`{enabled: true, reserveTokens: 16384, keepRecentTokens: 20000}`。
pub const DEFAULT_COMPACTION_SETTINGS: CompactionSettings = CompactionSettings {
    enabled: true,
    reserve_tokens: 16384,
    keep_recent_tokens: 20000,
};

/// ref `compaction.ts:298`：图片按固定字符数计入估算。
const ESTIMATED_IMAGE_CHARS: usize = 4800;

/// ref `compaction.ts:162-164`：优先用 `totalTokens`，否则四项求和。
#[must_use]
pub fn calculate_context_tokens(usage: &Usage) -> u64 {
    if usage.total_tokens != 0 {
        return usage.total_tokens;
    }
    usage.input + usage.output + usage.cache_read + usage.cache_write
}

/// ref `compaction.ts:300-314` `estimateTextAndImageContentChars`。
fn estimate_text_and_image_content_chars(content: &UserContent) -> usize {
    match content {
        UserContent::Text(text) => text.chars().count(),
        UserContent::Blocks(blocks) => blocks
            .iter()
            .map(|block| match block {
                // ref：`block.type === "text" && block.text` 才计数（空串不加）。
                pi_ai::Content::Text { text, .. } if !text.is_empty() => text.chars().count(),
                pi_ai::Content::Image { .. } => ESTIMATED_IMAGE_CHARS,
                _not_counted => 0,
            })
            .sum(),
    }
}

/// ref `compaction.ts:320-371` `estimateTokens`：按 `chars/4` 向上取整。
#[must_use]
pub fn estimate_tokens(message: &Message) -> u64 {
    let chars: usize = match message {
        Message::System { content, extra, .. } => {
            let mut chars = estimate_text_and_image_content_chars(content);
            // ref `:327-331`：`system.sections` 里每个**非空**段落按自身长度计入。
            if let Some(sections) = extra.get("sections").and_then(|value| value.as_object()) {
                for section in sections.values() {
                    if let Some(text) = section.as_str() {
                        if !text.is_empty() {
                            chars += text.chars().count();
                        }
                    }
                }
            }
            // ref `:332`：`toolsAdded` 按 JSON 文本长度计入。
            if let Some(tools) = extra.get("toolsAdded") {
                chars += tools.to_string().chars().count();
            }
            chars
        }
        Message::User { content, .. } => estimate_text_and_image_content_chars(content),
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .map(|block| match block {
                pi_ai::Content::Text { text, .. } => text.chars().count(),
                pi_ai::Content::Thinking { thinking, .. } => thinking.chars().count(),
                pi_ai::Content::ToolCall {
                    name, arguments, ..
                } => {
                    // ref `:349`：`block.name.length + JSON.stringify(block.arguments).length`。
                    name.chars().count()
                        + serde_json::Value::Object(arguments.clone())
                            .to_string()
                            .chars()
                            .count()
                }
                _not_counted => 0,
            })
            .sum(),
        // ref `compaction.ts:354-358`：`custom` 按文本字符算。
        Message::Custom { content, .. } => estimate_text_and_image_content_chars(content),
        // ref `:363-367`：两个摘要按 `summary.length` 算（**不是**按 user 的文本字符）。
        Message::BranchSummary { summary, .. } | Message::CompactionSummary { summary, .. } => {
            summary.chars().count()
        }
        Message::ToolResult { content, .. } => content
            .iter()
            .map(|block| match block {
                pi_ai::message::ToolResultContent::Text { text, .. } if !text.is_empty() => {
                    text.chars().count()
                }
                pi_ai::message::ToolResultContent::Image { .. } => ESTIMATED_IMAGE_CHARS,
                _not_counted => 0,
            })
            .sum(),
    };
    (chars as u64).div_ceil(4)
}

/// ref `compaction.ts:170-183`：只认"非 aborted / 非 error / 非全零"的助手 usage。
fn get_assistant_usage(message: &Message) -> Option<&Usage> {
    let Message::Assistant(assistant) = message else {
        return None;
    };
    if matches!(
        assistant.stop_reason,
        pi_ai::StopReason::Aborted | pi_ai::StopReason::Error
    ) {
        return None;
    }
    if calculate_context_tokens(&assistant.usage) == 0 {
        return None;
    }
    Some(&assistant.usage)
}

/// ref `compaction.ts:188-197`：从后往前找第一条带有效 usage 的助手消息。
#[must_use]
pub fn get_last_assistant_usage(messages: &[Message]) -> Option<Usage> {
    messages
        .iter()
        .rev()
        .find_map(|message| get_assistant_usage(message).copied())
}

/// ref `compaction.ts:206-212`：同时给出索引（估算尾随消息要用）。
fn get_last_assistant_usage_info(messages: &[Message]) -> Option<(Usage, usize)> {
    messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| get_assistant_usage(message).map(|usage| (*usage, index)))
}

/// ref `compaction.ts:199-204` `ContextUsageEstimate`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextUsageEstimate {
    pub tokens: u64,
    pub usage_tokens: u64,
    pub trailing_tokens: u64,
    pub last_usage_index: Option<usize>,
}

/// ref `compaction.ts:218-246` `estimateContextTokens`。
#[must_use]
pub fn estimate_context_tokens(messages: &[Message]) -> ContextUsageEstimate {
    let Some((usage, index)) = get_last_assistant_usage_info(messages) else {
        let estimated: u64 = messages.iter().map(estimate_tokens).sum();
        return ContextUsageEstimate {
            tokens: estimated,
            usage_tokens: 0,
            trailing_tokens: estimated,
            last_usage_index: None,
        };
    };
    let usage_tokens = calculate_context_tokens(&usage);
    let trailing_tokens: u64 = messages[index + 1..].iter().map(estimate_tokens).sum();
    ContextUsageEstimate {
        tokens: usage_tokens + trailing_tokens,
        usage_tokens,
        trailing_tokens,
        last_usage_index: Some(index),
    }
}

/// ref `compaction.ts:289-292` `shouldCompact`。
#[must_use]
pub fn should_compact(
    context_tokens: u64,
    context_window: u64,
    settings: &CompactionSettings,
) -> bool {
    if !settings.enabled {
        return false;
    }
    context_tokens > context_window.saturating_sub(settings.reserve_tokens)
}

/// ref `utils.ts:12-16` `FileOperations`：三类路径集合。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileOperations {
    pub read: std::collections::BTreeSet<String>,
    pub written: std::collections::BTreeSet<String>,
    pub edited: std::collections::BTreeSet<String>,
}

/// ref `utils.ts:18-24` `createFileOps`。
#[must_use]
pub fn create_file_ops() -> FileOperations {
    FileOperations::default()
}

/// ref `utils.ts:29-56` `extractFileOpsFromMessage`：
/// 只看**助手消息**里 `toolCall` 块的 `name`（`read`/`write`/`edit`）与 `arguments.path`。
pub fn extract_file_ops_from_message(message: &Message, file_ops: &mut FileOperations) {
    let Message::Assistant(assistant) = message else {
        return;
    };
    for block in &assistant.content {
        let pi_ai::Content::ToolCall {
            name, arguments, ..
        } = block
        else {
            continue;
        };
        let Some(path) = arguments.get("path").and_then(|value| value.as_str()) else {
            continue;
        };
        match name.as_str() {
            "read" => {
                file_ops.read.insert(path.to_string());
            }
            "write" => {
                file_ops.written.insert(path.to_string());
            }
            "edit" => {
                file_ops.edited.insert(path.to_string());
            }
            _not_a_file_tool => {}
        }
    }
}

/// ref `utils.ts:62-67` `computeFileLists`：
/// `readFiles` = 只读未改的（排序），`modifiedFiles` = `edited ∪ written`（排序）。
#[must_use]
pub fn compute_file_lists(file_ops: &FileOperations) -> (Vec<String>, Vec<String>) {
    let modified: std::collections::BTreeSet<&String> = file_ops
        .edited
        .iter()
        .chain(file_ops.written.iter())
        .collect();
    let read_only = file_ops
        .read
        .iter()
        .filter(|path| !modified.contains(*path))
        .cloned()
        .collect();
    (read_only, modified.into_iter().cloned().collect())
}

/// ref `utils.ts:72-82` `formatFileOperations`：
/// 两段各自包 `<read-files>` / `<modified-files>`，段间 `\n\n`，非空时整体前缀 `\n\n`。
#[must_use]
pub fn format_file_operations(read_files: &[String], modified_files: &[String]) -> String {
    let mut sections: Vec<String> = Vec::new();
    if !read_files.is_empty() {
        sections.push(format!(
            "<read-files>\n{}\n</read-files>",
            read_files.join("\n")
        ));
    }
    if !modified_files.is_empty() {
        sections.push(format!(
            "<modified-files>\n{}\n</modified-files>",
            modified_files.join("\n")
        ));
    }
    if sections.is_empty() {
        return String::new();
    }
    format!("\n\n{}", sections.join("\n\n"))
}

/// 条目对"上下文消息"的贡献（对应 ref 的 `sessionEntryToContextMessages`）。
///
/// **范围**：cut 点判定只走 `Message` 条目（ref 的 `find*` 先跳过 `compaction`，
/// 再对消息条目取上下文消息）。`CustomMessage` / `BranchSummary` 的投影由 P5-3 的投影层决定，
/// **本步不猜**，一律返回空（已知限制，见 `notes/p5-1a.md`）。
/// compaction 纯逻辑的错误（P5-1e E3）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompactionError {
    /// 遇到当前投影层还不支持的条目类型：**显式拒绝**，不得静默丢弃。
    #[error("compaction 暂不支持 {kind} 条目的上下文投影（P5-3 的投影层落地前不得静默丢弃）")]
    UnsupportedEntry { kind: &'static str },
}

/// 条目对"上下文消息"的贡献（对应 ref 的 `sessionEntryToContextMessages`）。
///
/// **P5-1e E3 改判**：此前对 `CustomMessage`/`BranchSummary` 等**静默返回空**。
/// 现在只有两类给出确定答案：`Message` → 它自己；`thinking_level_change`/`model_change` → 确定不产生消息。
/// 其余（custom_message / branch_summary / compaction / context_edit / unknown）**显式报错**，
/// 因为它们的投影规则要由 P5-3 的投影层决定 —— 静默丢弃会让 cut 点判定悄悄算错。
pub fn session_entry_to_context_messages(
    entry: &SessionEntry,
) -> Result<Vec<Message>, CompactionError> {
    match entry {
        SessionEntry::Message { message, .. } => Ok(vec![message.clone()]),
        SessionEntry::ThinkingLevelChange { .. } | SessionEntry::ModelChange { .. } => {
            Ok(Vec::new())
        }
        // ref `session-manager.ts:453-457`：`custom_message` → 一条 `custom` 角色消息
        // （`createCustomMessage(entry.customType, entry.content ?? [], entry.display, entry.details, …)`）。
        // 注意：pi-rs 的条目把 `display`/`details` 放在 `extra`（`CustomMessage` 只声明了 customType/content）。
        SessionEntry::CustomMessage {
            custom_type,
            content,
            extra,
            ..
        } => {
            let content = serde_json::from_value::<UserContent>(content.clone())
                .unwrap_or_else(|_not_content| UserContent::Text(String::new()));
            Ok(vec![Message::Custom {
                custom_type: custom_type.clone(),
                content,
                display: extra
                    .get("display")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                details: extra
                    .get("details")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                timestamp: 0,
                extra: serde_json::Map::new(),
            }])
        }
        // ref `:458-460`：**有 summary 的** `branch_summary` → 一条 `branchSummary` 消息。
        SessionEntry::BranchSummary {
            summary, from_id, ..
        } => {
            if summary.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![Message::BranchSummary {
                    summary: summary.clone(),
                    from_id: Some(from_id.clone()),
                    timestamp: 0,
                    extra: serde_json::Map::new(),
                }])
            }
        }
        // ref `:461-464`：`compaction` → （可选 `systemMessage` +）一条 `compactionSummary` 消息。
        SessionEntry::Compaction {
            summary,
            tokens_before,
            extra,
            ..
        } => {
            let mut messages: Vec<Message> = Vec::new();
            if let Some(system_message) = extra.get("systemMessage") {
                if let Ok(message) = serde_json::from_value::<Message>(system_message.clone()) {
                    messages.push(message);
                }
            }
            messages.push(Message::CompactionSummary {
                summary: summary.clone(),
                tokens_before: *tokens_before,
                timestamp: 0,
                extra: serde_json::Map::new(),
            });
            Ok(messages)
        }
        // ref `session-manager.ts:461-465`：`context_edit` 条目不产生上下文消息（它改的是**目标条目**，
        // 由投影层的 `projectContextEntry` 应用）⇒ 确定的空向量，不是"不支持"。
        SessionEntry::ContextEdit { .. } => Ok(Vec::new()),
        // ref `:465` 的最后 `return []`：未知/非上下文条目一律不产生消息。
        SessionEntry::Unknown(_not_known) => Ok(Vec::new()),
    }
}

/// ref `compaction.ts:373-386` `isCutPointMessage`。
#[must_use]
pub fn is_cut_point_message(message: &Message) -> bool {
    match message {
        // ref `:373-386`：custom / branchSummary / compactionSummary 都是 cut point。
        Message::User { .. }
        | Message::Assistant(_)
        | Message::Custom { .. }
        | Message::BranchSummary { .. }
        | Message::CompactionSummary { .. } => true,
        Message::System { .. } | Message::ToolResult { .. } => false,
    }
}

/// ref `compaction.ts:388-401` `isTurnStartMessage`（toolResult / assistant 之外都算 turn 起点）。
#[must_use]
pub fn is_turn_start_message(message: &Message) -> bool {
    match message {
        // ref `:388-401`：custom / branchSummary / compactionSummary 都是 turn 起点。
        Message::User { .. }
        | Message::Custom { .. }
        | Message::BranchSummary { .. }
        | Message::CompactionSummary { .. } => true,
        Message::Assistant(_) | Message::ToolResult { .. } => false,
        // pi-rs 里 system 条目不在上下文消息序列中；保守判 false（ref 的 switch 也落到 false）。
        Message::System { .. } => false,
    }
}

/// ref `compaction.ts:403-408` `isTurnStartEntry`：`compaction` 条目一律不算。
pub fn is_turn_start_entry(entry: &SessionEntry) -> Result<bool, CompactionError> {
    if matches!(entry, SessionEntry::Compaction { .. }) {
        return Ok(false);
    }
    Ok(session_entry_to_context_messages(entry)?
        .iter()
        .any(is_turn_start_message))
}

/// ref `compaction.ts:416-428` `findValidCutPoints`。
pub fn find_valid_cut_points(
    entries: &[SessionEntry],
    start_index: usize,
    end_index: usize,
) -> Result<Vec<usize>, CompactionError> {
    let mut cut_points = Vec::new();
    for (offset, entry) in entries[start_index..end_index.min(entries.len())]
        .iter()
        .enumerate()
    {
        let index = start_index + offset;
        if matches!(entry, SessionEntry::Compaction { .. }) {
            continue;
        }
        if session_entry_to_context_messages(entry)?
            .iter()
            .any(is_cut_point_message)
        {
            cut_points.push(index);
        }
    }
    Ok(cut_points)
}

/// ref `compaction.ts:434-441` `findTurnStartIndex`：从 `entryIndex` 往回找 turn 起点，找不到为 `None`。
pub fn find_turn_start_index(
    entries: &[SessionEntry],
    entry_index: usize,
    start_index: usize,
) -> Result<Option<usize>, CompactionError> {
    if entry_index >= entries.len() {
        return Ok(None);
    }
    for index in (start_index..=entry_index).rev() {
        if is_turn_start_entry(&entries[index])? {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

/// ref `compaction.ts:443-450` `CutPointResult`（`turnStartIndex` 用 `None` 表达 ref 的 `-1`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutPointResult {
    pub first_kept_entry_index: usize,
    pub turn_start_index: Option<usize>,
    pub is_split_turn: bool,
}

/// ref `compaction.ts:468-523` `findCutPoint`。
pub fn find_cut_point(
    entries: &[SessionEntry],
    start_index: usize,
    end_index: usize,
    keep_recent_tokens: u64,
) -> Result<CutPointResult, CompactionError> {
    let cut_points = find_valid_cut_points(entries, start_index, end_index)?;
    if cut_points.is_empty() {
        return Ok(CutPointResult {
            first_kept_entry_index: start_index,
            turn_start_index: None,
            is_split_turn: false,
        });
    }
    let mut accumulated_tokens: u64 = 0;
    let mut cut_index = cut_points[0];
    // ref：从最新往回累加，超过预算就取"不早于该位置"的最近有效切点。
    for index in (start_index..end_index.min(entries.len())).rev() {
        let message_tokens: u64 = session_entry_to_context_messages(&entries[index])?
            .iter()
            .map(estimate_tokens)
            .sum();
        if message_tokens == 0 {
            continue;
        }
        accumulated_tokens += message_tokens;
        if accumulated_tokens >= keep_recent_tokens {
            cut_index = cut_points
                .iter()
                .copied()
                .find(|candidate| *candidate >= index)
                .unwrap_or_else(|| *cut_points.last().unwrap_or(&cut_points[0]));
            break;
        }
    }
    // ref：往前吸收"不影响上下文"的元数据条目（compaction 边界或非空条目即停）。
    while cut_index > start_index {
        let previous = &entries[cut_index - 1];
        if matches!(previous, SessionEntry::Compaction { .. })
            || !session_entry_to_context_messages(previous)?.is_empty()
        {
            break;
        }
        cut_index -= 1;
    }
    let starts_turn = is_turn_start_entry(&entries[cut_index])?;
    let turn_start_index = if starts_turn {
        None
    } else {
        find_turn_start_index(entries, cut_index, start_index)?
    };
    Ok(CutPointResult {
        first_kept_entry_index: cut_index,
        turn_start_index,
        is_split_turn: !starts_turn && turn_start_index.is_some(),
    })
}

/// JS `JSON.stringify` 对单个值的等价实现（P5-1e E2；只覆盖 compaction 序列化用到的类型）。
///
/// 与 serde 默认输出的差异只在**数字**：`1.0` → `1`、`1e21` → `1e+21`、`-0` → `0`、
/// `|x| >= 1e21` 或 `< 1e-6` 用指数记法（正指数带 `+`）。字符串/布尔/null/数组/对象的形态与 serde 一致
/// （serde 也不转义 U+2028/DEL，控制字符转成 `\\u00XX`，与 JS 相同）。
#[must_use]
pub fn js_stringify(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Number(number) => js_number_to_string(number),
        serde_json::Value::String(text) => {
            serde_json::to_string(text).unwrap_or_else(|_not_serializable| "\"\"".to_string())
        }
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(js_stringify).collect();
            format!("[{}]", parts.join(","))
        }
        serde_json::Value::Object(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key)
                            .unwrap_or_else(|_not_serializable| "\"\"".to_string()),
                        js_stringify(value)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

/// 数字的 JS 格式（见 [`js_stringify`]）。
fn js_number_to_string(number: &serde_json::Number) -> String {
    if let Some(integer) = number.as_i64() {
        return integer.to_string();
    }
    if let Some(unsigned) = number.as_u64() {
        return unsigned.to_string();
    }
    let Some(float) = number.as_f64() else {
        return "0".to_string();
    };
    if float == 0.0 {
        // 含 `-0`（JS 里 `JSON.stringify(-0)` 是 `"0"`）
        return "0".to_string();
    }
    if !float.is_finite() {
        // JS 里 `JSON.stringify(Infinity/NaN)` 都是 `"null"`
        return "null".to_string();
    }
    let magnitude = float.abs();
    if !(1e-6..1e21).contains(&magnitude) {
        let formatted = format!("{float:e}");
        return match formatted.split_once('e') {
            Some((mantissa, exponent)) if exponent.starts_with('-') => {
                format!("{mantissa}e{exponent}")
            }
            Some((mantissa, exponent)) => format!("{mantissa}e+{exponent}"),
            None => formatted,
        };
    }
    format!("{float}")
}

/// ref `utils.ts:89`：摘要里工具结果的最大字符数。
pub const TOOL_RESULT_MAX_CHARS: usize = 2000;

/// ref `utils.ts:95-99` `truncateForSummary`（超出就保留前 `max_chars` 并附截断标记）。
#[must_use]
pub fn truncate_for_summary(text: &str, max_chars: usize) -> String {
    let length = text.chars().count();
    if length <= max_chars {
        return text.to_string();
    }
    let kept: String = text.chars().take(max_chars).collect();
    format!(
        "{kept}\n\n[... {} more characters truncated]",
        length - max_chars
    )
}

/// TS `contentText(content, separator)`（`pi-ai/dist/utils/text.js:2-9`）：
/// 字符串原样；块数组只取 `text` 块并用 `separator` 连接。`serializeConversation` 用**空串**。
fn content_text(content: &UserContent, separator: &str) -> String {
    match content {
        UserContent::Text(text) => text.clone(),
        UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                pi_ai::Content::Text { text, .. } => Some(text.as_str()),
                _not_text => None,
            })
            .collect::<Vec<_>>()
            .join(separator),
    }
}

/// ref `utils.ts:109-150` `serializeConversation`：把消息拼成"给摘要模型读"的文本。
///
/// 只处理 user / assistant / toolResult（system 不参与，与 ref 一致）；块之间用 `\n\n` 连接。
///
/// **已知限制**：工具参数的键序——ref 用 `Object.entries`（**插入序**），而 pi-rs 的
/// `ToolCall.arguments` 是 `serde_json::Map`（未开 `preserve_order` ⇒ **键名字典序**）。
/// 单参数工具调用不受影响；多参数时顺序可能不同（见 `notes/p5-1c.md`）。
#[must_use]
pub fn serialize_conversation(messages: &[Message]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for message in messages {
        match message {
            Message::User { content, .. } => {
                let text = content_text(content, "");
                if !text.is_empty() {
                    parts.push(format!("[User]: {text}"));
                }
            }
            Message::Assistant(assistant) => {
                let mut thinking_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<String> = Vec::new();
                for block in &assistant.content {
                    match block {
                        pi_ai::Content::Thinking { thinking, .. } => {
                            thinking_parts.push(thinking.clone());
                        }
                        pi_ai::Content::ToolCall {
                            name, arguments, ..
                        } => {
                            let args = arguments
                                .iter()
                                .map(|(key, value)| format!("{key}={}", js_stringify(value)))
                                .collect::<Vec<_>>()
                                .join(", ");
                            tool_calls.push(format!("{name}({args})"));
                        }
                        _not_serialized => {}
                    }
                }
                if !thinking_parts.is_empty() {
                    parts.push(format!(
                        "[Assistant thinking]: {}",
                        thinking_parts.join("\n")
                    ));
                }
                // ref `:135-137`：只要**存在** text 块就输出一行（内容可能为空串）。
                if assistant
                    .content
                    .iter()
                    .any(|block| matches!(block, pi_ai::Content::Text { .. }))
                {
                    parts.push(format!(
                        "[Assistant]: {}",
                        content_text(&UserContent::Blocks(assistant.content.clone()), "")
                    ));
                }
                if !tool_calls.is_empty() {
                    parts.push(format!("[Assistant tool calls]: {}", tool_calls.join("; ")));
                }
            }
            Message::ToolResult { content, .. } => {
                let blocks: Vec<pi_ai::Content> = content
                    .iter()
                    .filter_map(|block| match block {
                        pi_ai::message::ToolResultContent::Text { text, .. } => {
                            Some(pi_ai::Content::Text {
                                text: text.clone(),
                                text_signature: None,
                            })
                        }
                        pi_ai::message::ToolResultContent::Image { .. } => None,
                    })
                    .collect();
                let text = content_text(&UserContent::Blocks(blocks), "");
                if !text.is_empty() {
                    parts.push(format!(
                        "[Tool result]: {}",
                        truncate_for_summary(&text, TOOL_RESULT_MAX_CHARS)
                    ));
                }
            }
            // ref `utils.ts:109-150` 的 `serializeConversation` 只处理 user/assistant/toolResult；
            // 三个 agent 层角色不参与序列化（保持与 ref 同形，不是"静默丢弃"）。
            Message::System { .. }
            | Message::Custom { .. }
            | Message::BranchSummary { .. }
            | Message::CompactionSummary { .. } => {}
        }
    }
    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// P5-2a：`prepareCompaction`（ref `compaction.ts:59-96` 与 `:894-958`）
// ---------------------------------------------------------------------------

/// ref `compaction.ts:59-87` `extractFileOperations`：
/// 先吃上一条 compaction 的 `details`（来自 pi 自己生成的，且非 hook），再从消息的 tool call 里提取。
#[must_use]
pub fn extract_file_operations(
    messages: &[Message],
    entries: &[SessionEntry],
    prev_compaction_index: Option<usize>,
) -> FileOperations {
    let mut file_ops = create_file_ops();
    if let Some(index) = prev_compaction_index {
        if let Some(SessionEntry::Compaction { extra, .. }) = entries.get(index) {
            let from_hook = extra
                .get("fromHook")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if !from_hook {
                if let Some(details) = extra.get("details") {
                    if let Some(read_files) = details.get("readFiles").and_then(|v| v.as_array()) {
                        for file in read_files.iter().filter_map(|v| v.as_str()) {
                            file_ops.read.insert(file.to_string());
                        }
                    }
                    if let Some(modified) = details.get("modifiedFiles").and_then(|v| v.as_array())
                    {
                        for file in modified.iter().filter_map(|v| v.as_str()) {
                            file_ops.edited.insert(file.to_string());
                        }
                    }
                }
            }
        }
    }
    for message in messages {
        extract_file_ops_from_message(message, &mut file_ops);
    }
    file_ops
}

/// ref `compaction.ts:51-57` `CompactionDetails`：摘要里附带的文件清单。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactionDetails {
    pub read_files: Vec<String>,
    pub modified_files: Vec<String>,
}

/// ref `compaction.ts:894-958` `prepareCompaction` 的结果（`CompactionPreparation`）。
#[derive(Debug, Clone, PartialEq)]
pub struct Preparation {
    pub first_kept_entry_id: String,
    pub messages_to_summarize: Vec<Message>,
    pub turn_prefix_messages: Vec<Message>,
    pub is_split_turn: bool,
    pub tokens_before: u64,
    pub previous_summary: Option<String>,
    pub file_ops: FileOperations,
    pub settings: CompactionSettings,
}

/// ref `compaction.ts:894-958` `prepareCompaction`。
///
/// 返回 `Ok(None)` 的情形与 ref 的 `undefined` 一一对应：
/// ① 路径最后一条已经是 `compaction`；② 切点条目没有 id；③ 待摘要消息与 turn 前缀都为空。
pub fn prepare_compaction(
    path_entries: &[SessionEntry],
    settings: &CompactionSettings,
) -> Result<Option<Preparation>, CompactionError> {
    if matches!(path_entries.last(), Some(SessionEntry::Compaction { .. })) {
        return Ok(None);
    }
    let projection = crate::agent::projection::build_session_projection(
        path_entries,
        crate::agent::projection::LeafSelector::Unspecified,
    )?;
    let source_entries: Vec<SessionEntry> = projection
        .entries
        .iter()
        .map(|entry| entry.source_entry.clone())
        .collect();
    // ref `:907-916`：最新的 compaction 在投影里排在最前；更早的那些（原始范围命中）贡献为空消息。
    let prev_compaction_index = projection.entries.iter().position(|entry| {
        matches!(entry.source_entry, SessionEntry::Compaction { .. }) && !entry.messages.is_empty()
    });
    let (previous_summary, boundary_start) = match prev_compaction_index {
        Some(index) => (
            match &projection.entries[index].source_entry {
                SessionEntry::Compaction { summary, .. } => Some(summary.clone()),
                _not_compaction => None,
            },
            index + 1,
        ),
        None => (None, 0),
    };
    let boundary_end = projection.entries.len();
    let tokens_before =
        crate::agent::projection::estimate_projected_context_tokens(&projection, path_entries)
            .tokens;
    let cut_point = crate::agent::projection::find_projected_cut_point(
        &projection.entries,
        boundary_start,
        boundary_end,
        settings.keep_recent_tokens,
    )?;
    let Some(first_kept_entry) = projection.entries.get(cut_point.first_kept_entry_index) else {
        return Ok(None);
    };
    let Some(first_kept_entry_id) = first_kept_entry.source_entry.id() else {
        return Ok(None);
    };
    // ref `:925`：split turn 时，历史区间只到 turn 起点为止。
    let history_end = if cut_point.is_split_turn {
        cut_point.turn_start_index.unwrap_or(boundary_start)
    } else {
        cut_point.first_kept_entry_index
    };
    let collect = |range: &[crate::agent::projection::ProjectedSessionEntry]| -> Vec<Message> {
        range
            .iter()
            .flat_map(crate::agent::projection::get_messages_from_projected_entry_for_compaction)
            .collect()
    };
    let slice = |start: usize, end: usize| -> &[crate::agent::projection::ProjectedSessionEntry] {
        let end = end.min(projection.entries.len());
        if start >= end {
            return &[];
        }
        &projection.entries[start..end]
    };
    let messages_to_summarize = collect(slice(boundary_start, history_end));
    let turn_prefix_messages = if cut_point.is_split_turn {
        collect(slice(
            cut_point.turn_start_index.unwrap_or(boundary_start),
            cut_point.first_kept_entry_index,
        ))
    } else {
        Vec::new()
    };
    if messages_to_summarize.is_empty() && turn_prefix_messages.is_empty() {
        return Ok(None);
    }
    let mut file_ops = extract_file_operations(
        &messages_to_summarize,
        &source_entries,
        prev_compaction_index,
    );
    if cut_point.is_split_turn {
        for message in &turn_prefix_messages {
            extract_file_ops_from_message(message, &mut file_ops);
        }
    }
    Ok(Some(Preparation {
        first_kept_entry_id: first_kept_entry_id.to_string(),
        messages_to_summarize,
        turn_prefix_messages,
        is_split_turn: cut_point.is_split_turn,
        tokens_before,
        previous_summary,
        file_ops,
        settings: *settings,
    }))
}

// ---------------------------------------------------------------------------
// P5-2b/c：摘要请求的纯逻辑（提示词拼装、maxTokens、usage 合并、失败判定）
// ---------------------------------------------------------------------------

/// ref `compaction.ts:734-737`：`min(floor(0.8 * reserveTokens), model.maxTokens || ∞)`。
#[must_use]
pub fn summarization_max_tokens(reserve_tokens: u64, model_max_tokens: u64) -> u64 {
    let from_reserve = (reserve_tokens as f64 * 0.8).floor() as u64;
    if model_max_tokens > 0 {
        from_reserve.min(model_max_tokens)
    } else {
        from_reserve
    }
}

/// 摘要请求的提示词与上限（ref `compaction.ts:734-755` 的纯逻辑部分）。
#[derive(Debug, Clone, PartialEq)]
pub struct SummarizationPrompt {
    pub max_tokens: u64,
    /// 作为**单条 user 消息**发出去的文本（`<conversation>` 包裹 + 可选 `<previous-summary>` + 提示词）。
    pub prompt_text: String,
}

/// ref `compaction.ts:734-755`：拼出摘要请求的 user 文本与 `maxTokens`。
///
/// - `messages` 会先经 `convertToLlm`（本仓库的 [`to_llm_messages`]）再 `serializeConversation`；
/// - 有 `previous_summary` 时用 `UPDATE_SUMMARIZATION_PROMPT`，否则用 `SUMMARIZATION_PROMPT`；
/// - `custom_instructions` 追加为 `"\n\nAdditional focus: {…}"`。
#[must_use]
pub fn build_summarization_prompt(
    messages: &[Message],
    reserve_tokens: u64,
    model_max_tokens: u64,
    custom_instructions: Option<&str>,
    previous_summary: Option<&str>,
) -> SummarizationPrompt {
    use crate::agent::compaction_prompts::{update_summarization_prompt, SUMMARIZATION_PROMPT};
    let max_tokens = summarization_max_tokens(reserve_tokens, model_max_tokens);
    let mut base_prompt = if previous_summary.is_some() {
        update_summarization_prompt()
    } else {
        SUMMARIZATION_PROMPT.to_string()
    };
    if let Some(instructions) = custom_instructions {
        base_prompt = format!("{base_prompt}\n\nAdditional focus: {instructions}");
    }
    let prompt_text = build_conversation_wrapped_prompt(messages, &base_prompt, previous_summary);
    SummarizationPrompt {
        max_tokens,
        prompt_text,
    }
}

/// ref `compaction.ts:747-755` 的文本拼装（独立出来是为了让 split-turn 的
/// `TURN_PREFIX_SUMMARIZATION_PROMPT` 走同一套包裹逻辑，也能单独对拍）。
///
/// 形态：`<conversation>` 包裹序列化文本 + 空行；有前一份摘要时再加一个 `<previous-summary>` 段；
/// 最后接 `base_prompt`（见 `:751-755`）。
#[must_use]
pub fn build_conversation_wrapped_prompt(
    messages: &[Message],
    base_prompt: &str,
    previous_summary: Option<&str>,
) -> String {
    let llm_messages = crate::message_to_llm_for_summary(messages);
    let conversation_text = serialize_conversation(&llm_messages);
    let mut prompt_text = format!("<conversation>\n{conversation_text}\n</conversation>\n\n");
    if let Some(summary) = previous_summary {
        prompt_text.push_str(&format!(
            "<previous-summary>\n{summary}\n</previous-summary>\n\n"
        ));
    }
    prompt_text.push_str(base_prompt);
    prompt_text
}

/// ref `compaction.ts:115-136` `combineUsage`：逐项相加；`cacheWrite1h`/`reasoning`
/// **只有至少一侧有值时才出现**（与 TS 的可选字段语义一致）。
#[must_use]
pub fn combine_usage(first: &pi_ai::Usage, second: &pi_ai::Usage) -> pi_ai::Usage {
    let optional_sum = |left: Option<u64>, right: Option<u64>| -> Option<u64> {
        match (left, right) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        }
    };
    pi_ai::Usage {
        input: first.input + second.input,
        output: first.output + second.output,
        cache_read: first.cache_read + second.cache_read,
        cache_write: first.cache_write + second.cache_write,
        cache_write_1h: optional_sum(first.cache_write_1h, second.cache_write_1h),
        reasoning: optional_sum(first.reasoning, second.reasoning),
        total_tokens: first.total_tokens + second.total_tokens,
        cost: pi_ai::Cost {
            input: first.cost.input + second.cost.input,
            output: first.cost.output + second.cost.output,
            cache_read: first.cost.cache_read + second.cost.cache_read,
            cache_write: first.cost.cache_write + second.cost.cache_write,
            total: first.cost.total + second.cost.total,
        },
    }
}

/// ref `compaction.ts:607-615` `getSummarizationFailure`：
/// 摘要响应只要 `error` 或 `length` 就**不能**当会话检查点（`length` 时摘要不完整）。
#[must_use]
pub fn get_summarization_failure(
    stop_reason: &pi_ai::StopReason,
    error_message: Option<&str>,
    label: &str,
) -> Option<String> {
    match stop_reason {
        pi_ai::StopReason::Error => Some(format!(
            "{label} failed: {}",
            error_message.unwrap_or("Unknown error")
        )),
        pi_ai::StopReason::Length => Some(format!(
            "{label} failed: generation hit the token cap and the summary is incomplete"
        )),
        _not_a_failure => None,
    }
}

/// 摘要请求的选项（P5-2c；ref `compaction.ts:617-632` 的 `createSummarizationOptions`
/// 与 `:651-655` 的 `completeSummarization` 组装后的结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummarizationOptions {
    pub max_tokens: u64,
    /// ref `:628-630`：`model.reasoning && thinkingLevel && thinkingLevel !== "off"` 时带等级。
    pub reasoning: Option<String>,
    /// ref `:653`：摘要请求**显式关掉 prompt cache**（`cacheRetention: "none"`）。
    pub cache_retention_none: bool,
}

/// ref `compaction.ts:617-632` + `:651-655`：算出摘要请求要带的选项。
///
/// **未实现（已知限制）**：`sessionId`（ref `:654` 的 `options.sessionId ?? uuidv7()`）——
/// pi-rs 还没有"路由 id"这个概念，这里不发明；摘要请求的缓存控制已由
/// `cache_retention_none = true` 覆盖（ref 加 sessionId 的目的正是缓存路由）。
#[must_use]
pub fn summarization_options(
    max_tokens: u64,
    model_reasoning: bool,
    thinking_level: Option<&str>,
) -> SummarizationOptions {
    let reasoning = match thinking_level {
        Some(level) if model_reasoning && level != "off" => Some(level.to_string()),
        _not_enabled => None,
    };
    SummarizationOptions {
        max_tokens,
        reasoning,
        cache_retention_none: true,
    }
}

/// 阈值压缩的判定结果（ref `agent-session.ts:2699-2734`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdOutcome {
    /// 越过阈值 ⇒ 需要压缩（ref `:2731-2732` 调 `_runAutoCompaction("threshold", false)`）。
    Compact,
    /// 不需要压缩（含 ref `:2719-2725` 的"usage 比最近一次 compaction 还旧 ⇒ 直接 return false"）。
    Skip,
}

/// ref `agent-session.ts:2699-2734`：算出用于阈值判定的 `contextTokens`，并与阈值比较。
///
/// - 投影里**有 `context_edit`** ⇒ 用 `estimateProjectedContextTokens`（投影后的口径）；
/// - 助手消息 `stopReason === "error"` 或 usage 全零 ⇒ 用 `estimateContextTokens` 的消息体估算，
///   并按 ref `:2714-2725` 检查"usage 是否来自压缩之前"（陈旧 usage 不得立刻再次触发压缩）；
/// - 其余 ⇒ 直接用 usage 的 `calculateContextTokens`。
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn threshold_outcome(
    projection: &crate::agent::projection::SessionProjection,
    branch_entries: &[SessionEntry],
    assistant_usage: Option<&pi_ai::Usage>,
    is_error_stop: bool,
    context_window: u64,
    settings: &CompactionSettings,
    last_compaction_timestamp_ms: Option<i64>,
    last_usage_message_timestamp_ms: Option<i64>,
) -> ThresholdOutcome {
    let has_context_edits = projection
        .entries
        .iter()
        .any(|entry| matches!(entry.source_entry, SessionEntry::ContextEdit { .. }));
    let direct_context_tokens = assistant_usage.map(calculate_context_tokens).unwrap_or(0);
    let context_tokens = if has_context_edits {
        crate::agent::projection::estimate_projected_context_tokens(projection, branch_entries)
            .tokens
    } else if is_error_stop || direct_context_tokens == 0 {
        let estimate = estimate_context_tokens(&projection.messages);
        if let (Some(_usage_index), Some(compaction_ts), Some(usage_ts)) = (
            estimate.last_usage_index,
            last_compaction_timestamp_ms,
            last_usage_message_timestamp_ms,
        ) {
            // ref `:2719-2725`：usage 不比最近一次 compaction 新 ⇒ 是压缩前的陈旧用量 ⇒ 不压缩。
            if usage_ts <= compaction_ts {
                return ThresholdOutcome::Skip;
            }
        }
        estimate.tokens
    } else {
        direct_context_tokens
    };
    if should_compact(context_tokens, context_window, settings) {
        ThresholdOutcome::Compact
    } else {
        ThresholdOutcome::Skip
    }
}

/// ref `compaction.ts:1112-1115`：turn-prefix 摘要的上限用 **0.5** 系数（主摘要是 0.8）。
#[must_use]
pub fn turn_prefix_max_tokens(reserve_tokens: u64, model_max_tokens: u64) -> u64 {
    let from_reserve = (reserve_tokens as f64 * 0.5).floor() as u64;
    if model_max_tokens > 0 {
        from_reserve.min(model_max_tokens)
    } else {
        from_reserve
    }
}

/// ref `compaction.ts:1116-1118`：turn-prefix 请求的正文（`# Conversation` + `# Instructions` 两段）。
#[must_use]
pub fn build_turn_prefix_prompt(messages: &[Message]) -> String {
    use crate::agent::compaction_prompts::TURN_PREFIX_SUMMARIZATION_PROMPT;
    let llm_messages = pi_ai::message::to_llm_messages(messages);
    let conversation_text = serialize_conversation(&llm_messages);
    format!(
        "# Conversation\n{conversation_text}\n\n# Instructions\n{TURN_PREFIX_SUMMARIZATION_PROMPT}"
    )
}

/// ref `compaction.ts:104-114` `CompactionResult`（`estimatedTokensAfter` 由 P5-3 的流程层填，本步不产）。
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionResult {
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    pub details: CompactionDetails,
    /// 生成摘要的 LLM 调用的用量（split-turn 时是两次调用的合并）。
    pub usage: pi_ai::Usage,
}

/// 一次摘要调用（生产里 = `generateSummaryWithUsage`：发请求、判失败、取文本与 usage）。
/// 注入它是为了让 `compact()` 的**编排逻辑**（两次调用、拼接、用量合并）可离线单测。
pub type Summarizer<'a> = dyn FnMut(&str, u64) -> Result<(String, pi_ai::Usage), String> + 'a;

/// ref `compaction.ts:987-1096` `compact()`：按 `preparation` 生成摘要并返回 `CompactionResult`。
///
/// - split-turn（且有 turn 前缀消息）：先（若有待摘要历史）用主/更新提示词生成历史摘要，
///   再用 `TURN_PREFIX_SUMMARIZATION_PROMPT` 生成前缀摘要，最后按 ref `:1054` 拼接；
///   用量按 `:1055` 的 `combineUsage` 合并（历史那次没有调用时直接取前缀那次）。
/// - 其余：一次主/更新摘要调用。
///
/// `details` 由 `computeFileLists(fileOps)` 得出（ref `:939` 的 fileOps → 摘要里的文件清单）。
pub fn compact(
    preparation: &Preparation,
    model_max_tokens: u64,
    custom_instructions: Option<&str>,
    summarize: &mut Summarizer<'_>,
) -> Result<CompactionResult, String> {
    let settings = preparation.settings;
    let (summary, usage) = if is_split_turn(preparation) {
        let mut history_text = preparation
            .previous_summary
            .clone()
            .unwrap_or_else(|| "No prior history.".to_string());
        let mut history_usage: Option<pi_ai::Usage> = None;
        if !preparation.messages_to_summarize.is_empty() {
            let prompt = build_summarization_prompt(
                &preparation.messages_to_summarize,
                settings.reserve_tokens,
                model_max_tokens,
                custom_instructions,
                preparation.previous_summary.as_deref(),
            );
            let (text, usage) = summarize(&prompt.prompt_text, prompt.max_tokens)?;
            history_text = text;
            history_usage = Some(usage);
        }
        let prefix_prompt = build_turn_prefix_prompt(&preparation.turn_prefix_messages);
        let prefix_max = turn_prefix_max_tokens(settings.reserve_tokens, model_max_tokens);
        let (prefix_text, prefix_usage) = summarize(&prefix_prompt, prefix_max)?;
        // ref `:1054-1055`：拼接模板与用量合并。
        (
            stitch_summary(&history_text, &prefix_text),
            match history_usage {
                Some(history) => combine_usage(&history, &prefix_usage),
                None => prefix_usage,
            },
        )
    } else {
        let prompt = build_summarization_prompt(
            &preparation.messages_to_summarize,
            settings.reserve_tokens,
            model_max_tokens,
            custom_instructions,
            preparation.previous_summary.as_deref(),
        );
        summarize(&prompt.prompt_text, prompt.max_tokens)?
    };
    Ok(finish_compaction(summary, preparation, usage))
}

/// split-turn 走两条摘要路径的判定（ref `:1016`：`isSplitTurn && turnPrefixMessages.length > 0`）。
#[must_use]
pub fn is_split_turn(preparation: &Preparation) -> bool {
    preparation.is_split_turn && !preparation.turn_prefix_messages.is_empty()
}

/// ref `:1054` 的拼接模板：历史部分 + 分隔行 + `**Turn Context (split turn):**` + 前缀摘要。
#[must_use]
pub fn stitch_summary(history_text: &str, prefix_text: &str) -> String {
    format!("{history_text}\n\n---\n\n**Turn Context (split turn):**\n\n{prefix_text}")
}

/// 收尾：按 ref `:1078-1080` 把文件清单**追加**到摘要末尾（空清单时 `formatFileOperations`
/// 返回空串，等于不追加），并按 `:1086-1092` 组装结果。golden/S21 的 `summary` 末尾就带
/// `<read-files>…</read-files>`。`compact()` 与 `compact_with_provider()` 共用它。
#[must_use]
pub fn finish_compaction(
    summary: String,
    preparation: &Preparation,
    usage: pi_ai::Usage,
) -> CompactionResult {
    let (read_files, modified_files) = compute_file_lists(&preparation.file_ops);
    let file_section = format_file_operations(&read_files, &modified_files);
    CompactionResult {
        summary: format!("{summary}{file_section}"),
        first_kept_entry_id: preparation.first_kept_entry_id.clone(),
        tokens_before: preparation.tokens_before,
        details: CompactionDetails {
            read_files,
            modified_files,
        },
        usage,
    }
}

/// 用 provider 发**一次**摘要请求（ref `compaction.ts:641-661` 的 `completeSummarization`
/// 与 `:768-787` 的调用/校验部分）。
///
/// 与 ref 的对应关系：
/// - 上下文：`buildSummarizationContext`（`:704-714`）= system 用 `SUMMARIZATION_SYSTEM_PROMPT`、
///   单条 user 消息装提示词；
/// - 选项：`summarization_options`（`:617-632`）——`reasoning` 走 `context.reasoning_effort`，
///   `cacheRetention: "none"` 对应"不要把 system 块标成可缓存"（pi-rs 的 `cache_control_system = false`）；
/// - 校验：`getSummarizationFailure`（`:607-615`）与"不许调用工具"（`:781-783`）；
/// - 文本：`contentText(response.content)`（`pi-ai` 的 `contentText`，默认用 `\n` 连接文本块）。
///
/// 返回 `(摘要文本, usage)`；失败一律 `Err`（调用方据此放弃这次压缩，不写入条目）。
/// 摘要请求重试的回调（ref `agent-session.ts:3339-3360` 的 `RetryCallbacks`）。
///
/// 三个事件与 golden/S13c 实测一致：`scheduled`（将重试）→ `attempt_start`（这次重试开始）
/// → `finished`（重试循环结束，成功或最终失败都发）。
pub trait SummarizationRetryObserver {
    /// 将要在 `delay_ms` 后重试（`attempt` 从 1 开始）。
    fn scheduled(&self, attempt: u32, max_attempts: u32, delay_ms: u64, error_message: &str);
    /// 这一次重试开始。
    fn attempt_start(&self);
    /// 重试循环结束。
    fn finished(&self);
}

/// 摘要请求的重试上限与基础退避（`DEFAULT_RETRY_POLICY`；golden/S13c 实测
/// `maxAttempts: 3`、首次 `delayMs: 2000`）。
pub const SUMMARIZATION_MAX_ATTEMPTS: u32 = 3;
pub const SUMMARIZATION_RETRY_DELAY_MS: u64 = 2000;

pub async fn summarize_with_provider(
    provider: &dyn pi_ai::Provider,
    model: &str,
    prompt_text: &str,
    options: &SummarizationOptions,
    observer: Option<&dyn SummarizationRetryObserver>,
) -> Result<(String, pi_ai::Usage), String> {
    let mut attempt = 0_u32;
    loop {
        attempt += 1;
        match summarize_once(provider, model, prompt_text, options).await {
            Ok(success) => {
                if attempt > 1 {
                    if let Some(observer) = observer {
                        observer.finished();
                    }
                }
                return Ok(success);
            }
            Err(error) => {
                if attempt >= SUMMARIZATION_MAX_ATTEMPTS {
                    if let Some(observer) = observer {
                        observer.finished();
                    }
                    return Err(error);
                }
                let delay_ms = SUMMARIZATION_RETRY_DELAY_MS * 2_u64.pow(attempt - 1);
                if let Some(observer) = observer {
                    // 重试事件里带的是**底层错误原文**（ref 的 retry 回调收到的是 provider 的错误，
                    // 包装文案只在最终失败时用）。golden/S13c 实测：
                    // `errorMessage: "Stream ended without finish_reason"`（无 "Summarization failed: " 前缀）。
                    let raw = error
                        .strip_prefix("Summarization failed: ")
                        .unwrap_or(&error);
                    observer.scheduled(attempt, SUMMARIZATION_MAX_ATTEMPTS, delay_ms, raw);
                    observer.attempt_start();
                }
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }
    }
}

/// 一次摘要请求（不含重试）。
async fn summarize_once(
    provider: &dyn pi_ai::Provider,
    model: &str,
    prompt_text: &str,
    options: &SummarizationOptions,
) -> Result<(String, pi_ai::Usage), String> {
    use crate::agent::compaction_prompts::SUMMARIZATION_SYSTEM_PROMPT;
    let user = Message::User {
        content: UserContent::Text(prompt_text.to_string()),
        timestamp: 0,
        extra: serde_json::Map::new(),
    };
    let mut context = pi_ai::Context::from_messages(
        Some(SUMMARIZATION_SYSTEM_PROMPT.to_string()),
        std::slice::from_ref(&user),
    );
    // ref `:653`：摘要请求**不带 prompt cache**。
    context.cache_control_system = false;
    // ref `:628-630`：`options.reasoning` 会作为思考等级下发。
    context.reasoning_effort = options.reasoning.clone();
    let request = pi_ai::ProviderRequest {
        model: model.to_string(),
        context,
        max_completion_tokens: options.max_tokens,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    };
    let mut stream = provider
        .stream(request)
        .await
        .map_err(|error| format!("摘要请求失败：{error}"))?;
    while futures::StreamExt::next(&mut stream).await.is_some() {}
    let message = stream
        .final_message()
        .map_err(|poisoned| format!("摘要请求的最终消息不可用：{poisoned}"))?
        .ok_or_else(|| "摘要请求没有产生最终消息".to_string())?;
    if let Some(failure) = get_summarization_failure(
        &message.stop_reason,
        message.error_message.as_deref(),
        "Summarization",
    ) {
        return Err(failure);
    }
    if message
        .content
        .iter()
        .any(|block| matches!(block, pi_ai::Content::ToolCall { .. }))
    {
        return Err("Summarization attempted to call a tool".to_string());
    }
    // ref `:785`：`contentText(response.content)` —— 文本块用 `\n` 连接。
    let text = message
        .content
        .iter()
        .filter_map(|block| match block {
            pi_ai::Content::Text { text, .. } => Some(text.as_str()),
            _not_text => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok((text, message.usage))
}

/// `compact()` 的 provider 版：编排与 `compact()` **完全一致**（同样的提示词、同样的
/// 拼接与用量合并、同样的文件清单收尾），只是把两次摘要调用换成真实请求
/// （`summarize_with_provider`）。生产路径用它；`compact()` 的闭包版留给纯逻辑单测。
/// 摘要请求的模型/提示参数（把形参收成一个结构，避免超长参数表）。
pub struct SummarizationRequest<'a> {
    pub model: &'a str,
    pub model_max_tokens: u64,
    pub custom_instructions: Option<&'a str>,
    pub model_reasoning: bool,
    pub thinking_level: Option<&'a str>,
}

pub async fn compact_with_provider(
    preparation: &Preparation,
    request: &SummarizationRequest<'_>,
    provider: &dyn pi_ai::Provider,
    observer: Option<&dyn SummarizationRetryObserver>,
) -> Result<CompactionResult, String> {
    let settings = preparation.settings;
    let (summary, usage) = if is_split_turn(preparation) {
        let mut history_text = preparation
            .previous_summary
            .clone()
            .unwrap_or_else(|| "No prior history.".to_string());
        let mut history_usage: Option<pi_ai::Usage> = None;
        if !preparation.messages_to_summarize.is_empty() {
            let prompt = build_summarization_prompt(
                &preparation.messages_to_summarize,
                settings.reserve_tokens,
                request.model_max_tokens,
                request.custom_instructions,
                preparation.previous_summary.as_deref(),
            );
            let options = summarization_options(
                prompt.max_tokens,
                request.model_reasoning,
                request.thinking_level,
            );
            let (text, usage) = summarize_with_provider(
                provider,
                request.model,
                &prompt.prompt_text,
                &options,
                observer,
            )
            .await?;
            history_text = text;
            history_usage = Some(usage);
        }
        let prefix_prompt = build_turn_prefix_prompt(&preparation.turn_prefix_messages);
        // ref `:1112-1115`：前缀摘要的预算用 0.5 系数（主摘要用 0.8）。
        let prefix_max = turn_prefix_max_tokens(settings.reserve_tokens, request.model_max_tokens);
        let options =
            summarization_options(prefix_max, request.model_reasoning, request.thinking_level);
        let (prefix_text, prefix_usage) =
            summarize_with_provider(provider, request.model, &prefix_prompt, &options, observer)
                .await?;
        (
            stitch_summary(&history_text, &prefix_text),
            match history_usage {
                Some(history) => combine_usage(&history, &prefix_usage),
                None => prefix_usage,
            },
        )
    } else {
        let prompt = build_summarization_prompt(
            &preparation.messages_to_summarize,
            settings.reserve_tokens,
            request.model_max_tokens,
            request.custom_instructions,
            preparation.previous_summary.as_deref(),
        );
        let options = summarization_options(
            prompt.max_tokens,
            request.model_reasoning,
            request.thinking_level,
        );
        summarize_with_provider(
            provider,
            request.model,
            &prompt.prompt_text,
            &options,
            observer,
        )
        .await?
    };
    Ok(finish_compaction(summary, preparation, usage))
}

/// `_checkCompaction` 的十步判定结果（ref `agent-session.ts:2599-2734`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoCompactionDecision {
    /// Case 2（`:2667-2669`）：溢出了但停因是 `stop` ⇒ 压缩、**不重试**。
    OverflowNoRetry,
    /// Case 1（`:2693-2696`）：溢出/可恢复截断且停因不是 `stop` ⇒ 压缩后重试一次。
    OverflowRetry,
    /// 已试过一次恢复又溢出（`:2671-2691`）⇒ 只发失败事件，不再压缩。
    OverflowExhausted,
    /// Case 3（`:2699-2734`）：阈值分支，交给 [`threshold_outcome`] 定夺。
    Threshold,
    /// 不压缩。
    Skip,
}

/// [`overflow_outcome`] 的输入（只读的会话/模型事实）。
pub struct OverflowInputs<'a> {
    pub assistant: &'a pi_ai::AssistantMessage,
    pub model_provider: &'a str,
    pub model_id: &'a str,
    pub context_window: u64,
    pub model_max_tokens: u64,
    pub projection: &'a crate::agent::projection::SessionProjection,
    pub path_entries: &'a [SessionEntry],
    pub settings: &'a CompactionSettings,
    /// 最新 `compaction` 条目的时间戳原文（`None` = 没有该条目）。
    pub last_compaction_timestamp: Option<&'a str>,
    /// ref `:2602`：`false` 时**不**跳过 aborted（提示词前的那次检查就是这么传的）。
    pub skip_aborted_check: bool,
    /// ref `:2671`：本轮是否已经试过一次"压缩后重试"。
    pub overflow_recovery_attempted: bool,
}

/// ref `agent-session.ts:2599-2734`：按十步顺序判定自动压缩走哪条路。
///
/// 每一步的守卫都不能省（换模型的溢出、压缩前的陈旧消息、aborted 的分流都各有反例测试）。
#[must_use]
pub fn overflow_outcome(input: &OverflowInputs<'_>) -> AutoCompactionDecision {
    let assistant = input.assistant;
    // :2605-2606
    if !input.settings.enabled {
        return AutoCompactionDecision::Skip;
    }
    // :2608-2609
    if input.skip_aborted_check && assistant.stop_reason == pi_ai::types::StopReason::Aborted {
        return AutoCompactionDecision::Skip;
    }
    // :2616-2618
    let same_model =
        assistant.provider == input.model_provider && assistant.model == input.model_id;
    // :2622-2627：压缩边界之前的助手消息不得再次触发（时间戳解析不了就按"没有该条目"处理）
    if let Some(timestamp) = input.last_compaction_timestamp {
        if let Some(millis) = crate::timefmt::iso8601_to_unix_millis(timestamp) {
            if assistant.timestamp <= millis {
                return AutoCompactionDecision::Skip;
            }
        }
    }
    // :2633-2640：助手消息对应哪条条目（身份→按消息相等倒序找），以及它是否还在投影里
    let assistant_entry_id = input.path_entries.iter().rev().find_map(|entry| {
        if let SessionEntry::Message { id, message, .. } = entry {
            if *message == pi_ai::message::Message::Assistant(Box::new(assistant.clone())) {
                Some(id.clone())
            } else {
                None
            }
        } else {
            None
        }
    });
    let assistant_is_projected = match &assistant_entry_id {
        None => true,
        Some(id) => input.projection.entries.iter().any(|projected| {
            projected.source_entry.id() == Some(id.as_str())
                && projected
                    .messages
                    .iter()
                    .any(|message| matches!(message, pi_ai::message::Message::Assistant(_)))
        }),
    };
    // :2641-2654：助手之后的条目与三个派生量
    let assistant_index = assistant_entry_id.as_ref().and_then(|id| {
        input
            .path_entries
            .iter()
            .position(|entry| entry.id() == Some(id.as_str()))
    });
    let entries_after: &[SessionEntry] = match assistant_index {
        Some(index) => &input.path_entries[index + 1..],
        None => &[],
    };
    let has_post_assistant_context_edit = entries_after
        .iter()
        .any(|entry| matches!(entry, SessionEntry::ContextEdit { .. }));
    let latest_assistant_edit = entries_after.iter().rev().find_map(|entry| {
        if let SessionEntry::ContextEdit {
            target_id,
            replacement,
            ..
        } = entry
        {
            if Some(target_id.as_str()) == assistant_entry_id.as_deref() {
                Some(replacement.clone())
            } else {
                None
            }
        } else {
            None
        }
    });
    // `latestAssistantEdit?.replacement !== null`：条目不存在时按 `undefined !== null` = true
    let assistant_retained = assistant_entry_id.is_none()
        || (!entries_after
            .iter()
            .any(|entry| matches!(entry, SessionEntry::Compaction { .. }))
            && latest_assistant_edit
                .as_ref()
                .is_none_or(|replacement| *replacement != serde_json::Value::Null));
    let usage_matches_projection = assistant_is_projected && !has_post_assistant_context_edit;
    // :2655-2661：三个谓词
    let explicit_overflow = assistant.stop_reason == pi_ai::types::StopReason::Error
        && pi_ai::overflow::is_context_overflow(assistant, None);
    let context_overflow = same_model
        && ((explicit_overflow && assistant_retained)
            || (usage_matches_projection
                && pi_ai::overflow::is_context_overflow(assistant, Some(input.context_window))));
    let recoverable_length = same_model
        && assistant_is_projected
        && pi_ai::overflow::is_recoverable_length(assistant, input.model_max_tokens);
    // :2662-2697
    if context_overflow || recoverable_length {
        let will_retry = assistant.stop_reason != pi_ai::types::StopReason::Stop;
        if !will_retry {
            return AutoCompactionDecision::OverflowNoRetry;
        }
        if input.overflow_recovery_attempted {
            return AutoCompactionDecision::OverflowExhausted;
        }
        return AutoCompactionDecision::OverflowRetry;
    }
    // :2699
    AutoCompactionDecision::Threshold
}
