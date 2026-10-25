//! 会话投影（P5-2pre；ref `core/session-manager.ts:390-573`）。
//!
//! 这是 compaction 的前置：`prepareCompaction` 全程基于投影（`compaction.ts:902/919/920/927-934`）。
//!
//! 本次落地的是**不依赖新消息角色**的部分：`buildSessionPath` / `getSessionContextSettings` /
//! `buildContextEntries` / `projectContextEntry`（含 `context_edit` 的替换与移除）/ `buildSessionProjection`。
//! `custom_message` / `branch_summary` 的上下文消息仍由 [`session_entry_to_context_messages`] 决定
//! （在 `pi_ai::Message` 加上三个角色之前，它对这两类仍返回显式错误——不静默丢弃）。
use crate::agent::compaction::{
    session_entry_to_context_messages, CompactionError, CutPointResult,
};
use crate::session::SessionEntry;
use pi_ai::message::Message;

/// 投影后的一条条目（ref `session-manager.ts:209` 的 `ProjectedSessionEntry`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedSessionEntry {
    pub source_entry: SessionEntry,
    /// 该条目对模型上下文的消息（`context_edit` 已应用）。
    pub messages: Vec<Message>,
}

/// 投影结果（ref `:216` 的 `SessionProjection`）。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionProjection {
    pub entries: Vec<ProjectedSessionEntry>,
    pub messages: Vec<Message>,
    pub thinking_level: String,
    /// 最后一条 `model_change` 或助手消息里的 `(provider, modelId)`。
    pub model: Option<(String, String)>,
}

/// 会话里**最后一份**已声明的工具（`toolsAdded` 的 JSON 数组）。
///
/// 语义照 ref `packages/ai/src/utils/transcript.ts:170-175` 的 `getDeclaredTools`：
/// 逐条 system 消息收集 `toolsAdded`，**后者覆盖前者**；`compaction` 条目的
/// `systemMessage` 经投影后也算一条 system 消息，所以同样计入。
/// 用途：`toolsRemoved` 的差分依据（`getToolStateChanges` 的 `previous`）。
#[must_use]
pub fn last_declared_tools(entries: &[SessionEntry]) -> Vec<serde_json::Value> {
    let mut declared = Vec::new();
    if let Ok(projection) = build_session_projection(entries, LeafSelector::Unspecified) {
        for message in &projection.messages {
            if let Message::System { extra, .. } = message {
                if let Some(serde_json::Value::Array(tools)) = extra.get("toolsAdded") {
                    declared = tools.clone();
                }
            }
        }
    }
    declared
}

/// `buildSessionPath` 的叶子选择（ref `:395-403`：`leafId === null` → 空路径；给了就找，找不到用最后一条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafSelector<'a> {
    /// 没给（等价于 JS 的 `undefined`）。
    Unspecified,
    /// 显式 `null`（ref 里表示"空路径"）。
    Null,
    Id(&'a str),
}

fn index_of(entries: &[SessionEntry], id: &str) -> Option<usize> {
    entries.iter().position(|entry| entry.id() == Some(id))
}

/// ref `session-manager.ts:390-416` `buildSessionPath`：沿 `parentId` 上溯再反转。
#[must_use]
pub fn build_session_path(entries: &[SessionEntry], leaf: LeafSelector<'_>) -> Vec<SessionEntry> {
    if matches!(leaf, LeafSelector::Null) {
        return Vec::new();
    }
    let leaf_index = match leaf {
        LeafSelector::Id(id) => index_of(entries, id),
        LeafSelector::Unspecified | LeafSelector::Null => None,
    };
    // ref `:403`：找不到（或没给）就用最后一条。
    let mut current = leaf_index.or_else(|| entries.len().checked_sub(1));
    let mut path: Vec<SessionEntry> = Vec::new();
    let mut seen: Vec<usize> = Vec::new();
    while let Some(index) = current {
        // 环保护（ref 没有；这里只防死循环，不改变有向无环输入的结果）。
        if seen.contains(&index) {
            break;
        }
        seen.push(index);
        path.push(entries[index].clone());
        current = entries[index]
            .parent_id()
            .and_then(|parent| index_of(entries, parent));
    }
    path.reverse();
    path
}

/// ref `session-manager.ts:418-433` `getSessionContextSettings`。
#[must_use]
pub fn get_session_context_settings(path: &[SessionEntry]) -> (String, Option<(String, String)>) {
    let mut thinking_level = "off".to_string();
    let mut model: Option<(String, String)> = None;
    for entry in path {
        match entry {
            SessionEntry::ThinkingLevelChange {
                thinking_level: level,
                ..
            } => {
                thinking_level = level.clone();
            }
            SessionEntry::ModelChange {
                provider, model_id, ..
            } => {
                model = Some((provider.clone(), model_id.clone()));
            }
            // ref `:427-429`：助手消息里的 `(provider, model)` 也算模型设置。
            SessionEntry::Message {
                message: Message::Assistant(assistant),
                ..
            } => {
                model = Some((assistant.provider.clone(), assistant.model.clone()));
            }
            _not_settings_bearing => {}
        }
    }
    (thinking_level, model)
}

/// ref `session-manager.ts:476-512` `buildContextEntries`：
/// 取路径上最后一条 `compaction`，用它的 `firstKeptEntryId` 保留尾部（跳过 system 消息）。
#[must_use]
pub fn build_context_entries(
    entries: &[SessionEntry],
    leaf: LeafSelector<'_>,
) -> Vec<SessionEntry> {
    let path = build_session_path(entries, leaf);
    let Some(compaction_index) = path
        .iter()
        .rposition(|entry| matches!(entry, SessionEntry::Compaction { .. }))
    else {
        return path;
    };
    let SessionEntry::Compaction {
        first_kept_entry_id,
        ..
    } = &path[compaction_index]
    else {
        return path;
    };
    let mut context_entries = vec![path[compaction_index].clone()];
    let mut found_first_kept = false;
    for entry in &path[..compaction_index] {
        if entry.id() == Some(first_kept_entry_id.as_str()) {
            found_first_kept = true;
        }
        let is_system_message = matches!(
            entry,
            SessionEntry::Message {
                message: Message::System { .. },
                ..
            }
        );
        if found_first_kept && !is_system_message {
            context_entries.push(entry.clone());
        }
    }
    context_entries.extend(path[compaction_index + 1..].iter().cloned());
    context_entries
}

/// `context_edit` 对目标条目的作用（ref `:519-540`：`replacement === null` → 移除该条消息）。
#[derive(Debug, Clone, PartialEq)]
enum EditEffect<'a> {
    Remove,
    Replace(&'a serde_json::Value),
}

/// ref `:519-540` `projectContextEntry`：应用 `context_edit`，其余原样。
fn project_context_entry(
    entry: &SessionEntry,
    edit: Option<EditEffect<'_>>,
) -> Result<Vec<Message>, CompactionError> {
    let messages = session_entry_to_context_messages(entry)?;
    let Some(effect) = edit else {
        return Ok(messages);
    };
    match effect {
        EditEffect::Remove => Ok(Vec::new()),
        EditEffect::Replace(replacement) => {
            let content = replacement
                .get("content")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            Ok(messages
                .into_iter()
                .map(|message| apply_replacement(message, &content))
                .collect())
        }
    }
}

/// ref `:525-539`：只对 user/assistant/toolResult/custom 替换 `content`；
/// assistant/toolResult 且 `content` 是字符串时包成单个 text 块。
fn apply_replacement(message: Message, content: &serde_json::Value) -> Message {
    let wrapped = match (&message, content) {
        (Message::Assistant(_) | Message::ToolResult { .. }, serde_json::Value::String(text)) => {
            serde_json::json!([{ "type": "text", "text": text }])
        }
        _not_string_content => content.clone(),
    };
    let Ok(replaced) = serde_json::from_value::<pi_ai::message::UserContent>(wrapped) else {
        return message;
    };
    match message {
        Message::User {
            timestamp, extra, ..
        } => Message::User {
            content: replaced,
            timestamp,
            extra,
        },
        Message::Assistant(mut assistant) => {
            let blocks: Vec<pi_ai::Content> = match &replaced {
                pi_ai::message::UserContent::Blocks(blocks) => blocks.clone(),
                pi_ai::message::UserContent::Text(text) => vec![pi_ai::Content::Text {
                    text: text.clone(),
                    text_signature: None,
                }],
            };
            assistant.content = blocks;
            Message::Assistant(assistant)
        }
        Message::ToolResult {
            tool_call_id,
            tool_name,
            is_error,
            timestamp,
            extra,
            ..
        } => {
            let blocks = match replaced {
                pi_ai::message::UserContent::Blocks(blocks) => blocks,
                pi_ai::message::UserContent::Text(text) => vec![pi_ai::Content::Text {
                    text,
                    text_signature: None,
                }],
            };
            let content = blocks
                .into_iter()
                .filter_map(|block| match block {
                    pi_ai::Content::Text { text, .. } => {
                        Some(pi_ai::message::ToolResultContent::Text {
                            text,
                            extra: serde_json::Map::new(),
                        })
                    }
                    _not_text => None,
                })
                .collect();
            Message::ToolResult {
                tool_call_id,
                tool_name,
                content,
                is_error,
                timestamp,
                extra,
            }
        }
        other => other,
    }
}

/// ref `session-manager.ts:543-573` `buildSessionProjection`。
///
/// `context_edit` 按 `targetId` 取**最后一条**（ref `:551-554`）；`compaction` 条目排在 index 0 时贡献消息，
/// 更早的（`index > 0`）贡献空（ref `:561-563` 的注释解释了原因）。
pub fn build_session_projection(
    entries: &[SessionEntry],
    leaf: LeafSelector<'_>,
) -> Result<SessionProjection, CompactionError> {
    let path = build_session_path(entries, leaf);
    let (thinking_level, model) = get_session_context_settings(&path);
    let context_entries = build_context_entries(entries, leaf);
    let mut edits: Vec<(String, EditEffect<'_>)> = Vec::new();
    for entry in &context_entries {
        if let SessionEntry::ContextEdit {
            target_id,
            replacement,
            ..
        } = entry
        {
            let effect = if replacement.is_null() {
                EditEffect::Remove
            } else {
                EditEffect::Replace(replacement)
            };
            edits.retain(|(id, _not_effect)| id != target_id);
            edits.push((target_id.clone(), effect));
        }
    }
    let mut projected_entries: Vec<ProjectedSessionEntry> = Vec::new();
    for (index, source_entry) in context_entries.iter().enumerate() {
        let is_older_compaction =
            matches!(source_entry, SessionEntry::Compaction { .. }) && index > 0;
        let messages = if is_older_compaction {
            Vec::new()
        } else {
            let edit = edits
                .iter()
                .find(|(target, _not_effect)| source_entry.id() == Some(target.as_str()))
                .map(|(_target, effect)| effect.clone());
            project_context_entry(source_entry, edit)?
        };
        projected_entries.push(ProjectedSessionEntry {
            source_entry: source_entry.clone(),
            messages,
        });
    }
    let messages = projected_entries
        .iter()
        .flat_map(|entry| entry.messages.iter().cloned())
        .collect();
    Ok(SessionProjection {
        entries: projected_entries,
        messages,
        thinking_level,
        model,
    })
}

// ---------------------------------------------------------------------------
// 投影版的 cut 点与 token 估算（P5-2pre；ref `compaction.ts:249-284` 与 `:812-892`）
// ---------------------------------------------------------------------------

/// ref `compaction.ts:812-815` `isProjectedTurnStart`：`compaction` 条目不可能是 turn 起点。
#[must_use]
pub fn is_projected_turn_start(entry: &ProjectedSessionEntry) -> bool {
    if matches!(entry.source_entry, SessionEntry::Compaction { .. }) {
        return false;
    }
    entry
        .messages
        .iter()
        .any(crate::agent::compaction::is_turn_start_message)
}

/// ref `:817-822` `findProjectedTurnStartIndex`：从 `entry_index` 往回找，找不到给 `None`（ref 的 `-1`）。
#[must_use]
pub fn find_projected_turn_start_index(
    entries: &[ProjectedSessionEntry],
    entry_index: usize,
    start_index: usize,
) -> Option<usize> {
    if entry_index >= entries.len() {
        return None;
    }
    (start_index..=entry_index)
        .rev()
        .find(|index| is_projected_turn_start(&entries[*index]))
}

/// ref `:824-892` `findProjectedCutPoint`（含 `:856-878` 的"恢复省略后缀"规则）。
pub fn find_projected_cut_point(
    entries: &[ProjectedSessionEntry],
    start_index: usize,
    end_index: usize,
    keep_recent_tokens: u64,
) -> Result<CutPointResult, CompactionError> {
    let mut cut_points: Vec<usize> = Vec::new();
    for (offset, entry) in entries[start_index..end_index.min(entries.len())]
        .iter()
        .enumerate()
    {
        let index = start_index + offset;
        if matches!(entry.source_entry, SessionEntry::Compaction { .. }) {
            continue;
        }
        if entry
            .messages
            .iter()
            .any(crate::agent::compaction::is_cut_point_message)
        {
            cut_points.push(index);
        }
    }
    if cut_points.is_empty() {
        return Ok(CutPointResult {
            first_kept_entry_index: start_index,
            turn_start_index: None,
            is_split_turn: false,
        });
    }
    let mut accumulated_tokens: u64 = 0;
    let mut exceeded_budget = false;
    let mut cut_index = cut_points[0];
    for index in (start_index..end_index.min(entries.len())).rev() {
        let message_tokens: u64 = entries[index]
            .messages
            .iter()
            .map(crate::agent::compaction::estimate_tokens)
            .sum();
        if message_tokens == 0 {
            continue;
        }
        accumulated_tokens += message_tokens;
        if accumulated_tokens >= keep_recent_tokens {
            exceeded_budget = true;
            cut_index = cut_points
                .iter()
                .copied()
                .find(|candidate| *candidate >= index)
                .unwrap_or_else(|| *cut_points.last().unwrap_or(&cut_points[0]));
            break;
        }
    }
    // ref `:856-878`：只有"闭合的省略后缀 + 被省略的 assistant 尝试"才把 cut 推进一格。
    let suffix =
        &entries[cut_index.saturating_add(1).min(entries.len())..end_index.min(entries.len())];
    let intrinsically_visible = |entry: &ProjectedSessionEntry| -> Result<bool, CompactionError> {
        if matches!(entry.source_entry, SessionEntry::ContextEdit { .. }) {
            return Ok(false);
        }
        Ok(!session_entry_to_context_messages(&entry.source_entry)?.is_empty())
    };
    let mut omitted_ids: Vec<String> = Vec::new();
    for entry in suffix {
        if intrinsically_visible(entry)? && entry.messages.is_empty() {
            if let Some(id) = entry.source_entry.id() {
                omitted_ids.push(id.to_string());
            }
        }
    }
    let has_external_replacement = suffix.iter().any(|entry| match &entry.source_entry {
        SessionEntry::ContextEdit {
            target_id,
            replacement,
            ..
        } => !replacement.is_null() && !omitted_ids.contains(target_id),
        _not_an_edit => false,
    });
    let has_omitted_assistant = suffix.iter().any(|entry| {
        matches!(
            &entry.source_entry,
            SessionEntry::Message {
                message: Message::Assistant(_),
                ..
            }
        ) && intrinsically_visible(entry).unwrap_or(false)
            && entry.messages.is_empty()
    });
    let every_entry_closed = suffix.iter().all(|entry| {
        !matches!(entry.source_entry, SessionEntry::Compaction { .. })
            && (!intrinsically_visible(entry).unwrap_or(false) || entry.messages.is_empty())
    });
    if exceeded_budget && !has_external_replacement && has_omitted_assistant && every_entry_closed {
        cut_index += 1;
    }
    // ref `:880-884`：向前吸收"不影响上下文"的条目。
    while cut_index > start_index {
        let previous = &entries[cut_index - 1];
        if matches!(previous.source_entry, SessionEntry::Compaction { .. })
            || !previous.messages.is_empty()
        {
            break;
        }
        cut_index -= 1;
    }
    let starts_turn = is_projected_turn_start(&entries[cut_index]);
    let turn_start_index = if starts_turn {
        None
    } else {
        find_projected_turn_start_index(entries, cut_index, start_index)
    };
    Ok(CutPointResult {
        first_kept_entry_index: cut_index,
        turn_start_index,
        is_split_turn: !starts_turn && turn_start_index.is_some(),
    })
}

/// ref `compaction.ts:249-284` `estimateProjectedContextTokens`：
/// 若最后一条 usage 晚于最近的 `context_edit`/`compaction`，就信它；否则退化成"当前 system + 全部非 system 消息"的估算。
pub fn estimate_projected_context_tokens(
    projection: &SessionProjection,
    branch_entries: &[SessionEntry],
) -> crate::agent::compaction::ContextUsageEstimate {
    let estimate = crate::agent::compaction::estimate_context_tokens(&projection.messages);
    if let Some(last_usage_index) = estimate.last_usage_index {
        // 把投影消息索引映回"来源条目 id"（ref `:255-265`）。
        let mut projected_index = 0usize;
        let mut usage_entry_id: Option<String> = None;
        for entry in &projection.entries {
            let next_index = projected_index + entry.messages.len();
            if last_usage_index < next_index {
                usage_entry_id = entry.source_entry.id().map(str::to_string);
                break;
            }
            projected_index = next_index;
        }
        let usage_entry_index = usage_entry_id.as_deref().and_then(|id| {
            branch_entries
                .iter()
                .position(|entry| entry.id() == Some(id))
        });
        let latest_invalidating = branch_entries.iter().rposition(|entry| {
            matches!(
                entry,
                SessionEntry::ContextEdit { .. } | SessionEntry::Compaction { .. }
            )
        });
        let usage_is_after_invalidating = match (usage_entry_index, latest_invalidating) {
            (Some(usage), Some(invalidating)) => usage > invalidating,
            (Some(_usage), None) => true,
            (None, _not_found) => false,
        };
        if usage_is_after_invalidating {
            return estimate;
        }
    }
    // 退化路径（ref `:278-283`）：当前 system 一条 + 其余非 system 消息。
    let mut tokens: u64 = 0;
    for message in &projection.messages {
        match message {
            Message::System { .. } => {
                // ref 只算**当前** system（`getCurrentSystemMessage`）；这里退化成"计入每条 system"，
                // 因为 pi-rs 的投影里 system 至多一条（`Context.system` 与条目的分工）。
                tokens = tokens.saturating_add(crate::agent::compaction::estimate_tokens(message));
            }
            _not_system => {
                tokens = tokens.saturating_add(crate::agent::compaction::estimate_tokens(message));
            }
        }
    }
    crate::agent::compaction::ContextUsageEstimate {
        tokens,
        usage_tokens: 0,
        trailing_tokens: tokens,
        last_usage_index: None,
    }
}

/// ref `compaction.ts:97-101` `getMessagesFromProjectedEntryForCompaction`：
/// `compaction` 条目不给消息；其余条目**过滤掉 system 消息**（system 是提示状态，
/// 由 compaction 条目自己负责重放）。
#[must_use]
pub fn get_messages_from_projected_entry_for_compaction(
    entry: &ProjectedSessionEntry,
) -> Vec<Message> {
    if matches!(entry.source_entry, SessionEntry::Compaction { .. }) {
        return Vec::new();
    }
    entry
        .messages
        .iter()
        .filter(|message| !matches!(message, Message::System { .. }))
        .cloned()
        .collect()
}
