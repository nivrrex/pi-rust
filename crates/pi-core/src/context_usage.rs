//! 上下文用量（ref `core/agent-session.ts:3858-3902` 的 `getContextUsage`）。
//!
//! 为什么单独成模块：页脚百分比、压缩判定、扩展的 `ctx.usage` 三处都读它，
//! 而它的语义有一个容易漏掉的分支——**压缩之后、下一次助手响应之前，用量是"未知"
//! 而不是 0**。把"未知"写成 0 会让页脚显示 `0.0%`（用户以为上下文被清空了），
//! 写成 `?` 才是 ref 的行为（`footer.ts:150-156`）。
//!
//! 判定规则（ref `:3865-3889`）：
//! 1. 分支上**最近一次压缩**之后，若没有任何"可用助手消息"（`stopReason` 不是
//!    aborted/error，且 `calculateContextTokens(usage) > 0`）⇒ `tokens/percent` 为 `None`；
//! 2. 否则用 `estimateProjectedContextTokens` 估算（pi-rs 侧是
//!    `compaction::estimate_tokens(系统提示) + 投影里各消息的估算`）。
//!
//! 本模块只做判定与算式，token 估算由调用方传入（它需要系统提示文本，属装配层）。

use crate::session::SessionEntry;

/// 一次上下文用量的读取结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextUsage {
    /// 估算的 token 数；`None` = **未知**（压缩后还没有新的助手响应）。
    pub tokens: Option<u64>,
    /// 上下文窗口（> 0）。
    pub context_window: u64,
    /// 百分比；`None` = **未知**（与 `tokens` 同步）。
    pub percent: Option<f64>,
}

/// 分支上是否存在"最近一次压缩之后"的可用助手 usage（ref `:3872-3889`）。
///
/// `entries` 应是**当前分支**（叶子路径）；传整个文件会误把其它分支的助手消息算进来。
/// `projected_assistant_ids` 是投影里带"可用助手消息"的源条目 id 集合
/// （ref 的 `projectedAssistants`）。
#[must_use]
pub fn has_post_compaction_usage(
    branch: &[SessionEntry],
    projected_assistant_ids: &std::collections::HashSet<String>,
) -> bool {
    let Some(compaction_index) = branch
        .iter()
        .rposition(|entry| matches!(entry, SessionEntry::Compaction { .. }))
    else {
        // 没有压缩 ⇒ 不存在"压缩后无 usage"的问题（ref 该分支直接进入估算）。
        return true;
    };
    branch
        .iter()
        .skip(compaction_index + 1)
        .any(|entry| projected_assistant_ids.contains(entry_id(entry)))
}

/// 取条目的 id（复用 `SessionEntry::id`；未知类型没有 id ⇒ 返回空串，
/// 这样它永远不会与投影里的助手 id 相等，判定自然会忽略它）。
fn entry_id(entry: &SessionEntry) -> &str {
    entry.id().unwrap_or("")
}

/// 组装一次结果。
///
/// `tokens` 为 `None` ⇒ `percent` 也是 `None`（两者必须同步：只给百分比不给 token
/// 会让扩展侧 `ctx.usage.tokens` 与 `percent` 互相矛盾）。
#[must_use]
pub fn usage(tokens: Option<u64>, context_window: u64) -> ContextUsage {
    match tokens {
        Some(tokens) if context_window > 0 => ContextUsage {
            tokens: Some(tokens),
            context_window,
            percent: Some(tokens as f64 / context_window as f64 * 100.0),
        },
        // 窗口 <= 0 时 ref 直接返回 `undefined`（`:3862-3863`）⇒ 这里按"未知"处理。
        _other => ContextUsage {
            tokens: None,
            context_window,
            percent: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn compaction(id: &str) -> SessionEntry {
        SessionEntry::Compaction {
            id: id.to_string(),
            parent_id: None,
            timestamp: "t".to_string(),
            summary: "s".to_string(),
            first_kept_entry_id: "first".to_string(),
            tokens_before: 0,
            extra: serde_json::Map::new(),
        }
    }

    /// 一个"普通条目"占位（用 `context_edit`，它有 id 但不带 message）。
    fn marker(id: &str) -> SessionEntry {
        SessionEntry::ContextEdit {
            id: id.to_string(),
            parent_id: None,
            timestamp: "t".to_string(),
            target_id: "x".to_string(),
            replacement: serde_json::Value::Null,
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn no_compaction_means_usage_is_known() {
        let branch = vec![marker("a"), marker("b")];
        let ids = HashSet::new();
        assert!(has_post_compaction_usage(&branch, &ids), "无压缩 ⇒ 可估");
    }

    #[test]
    fn compaction_without_later_assistant_reports_unknown() {
        let branch = vec![marker("a"), compaction("c"), marker("b")];
        let ids: HashSet<String> = HashSet::new();
        assert!(
            !has_post_compaction_usage(&branch, &ids),
            "压缩之后没有可用助手 ⇒ 用量未知（ref `:3887-3889`）"
        );
    }

    #[test]
    fn compaction_followed_by_assistant_usage_is_known() {
        let branch = vec![marker("a"), compaction("c"), marker("b")];
        let ids: HashSet<String> = ["b".to_string()].into_iter().collect();
        assert!(
            has_post_compaction_usage(&branch, &ids),
            "压缩之后有可用助手 ⇒ 可估"
        );
    }

    #[test]
    fn assistant_before_compaction_does_not_count() {
        let branch = vec![marker("a"), compaction("c"), marker("b")];
        // `a` 在压缩**之前**，它的 usage 反映压缩前的上下文（ref `:3865-3867`）。
        let ids: HashSet<String> = ["a".to_string()].into_iter().collect();
        assert!(
            !has_post_compaction_usage(&branch, &ids),
            "压缩前的助手 usage 不能用来估算压缩后的上下文"
        );
    }

    #[test]
    fn only_the_last_compaction_matters() {
        let branch = vec![
            marker("a"),
            compaction("c1"),
            marker("b"),
            compaction("c2"),
            marker("d"),
        ];
        // 只有 `b`（第一次压缩之后、第二次压缩之前）有 usage ⇒ 仍未知。
        let ids: HashSet<String> = ["b".to_string()].into_iter().collect();
        assert!(
            !has_post_compaction_usage(&branch, &ids),
            "应以**最近一次**压缩为界"
        );
        let ids: HashSet<String> = ["d".to_string()].into_iter().collect();
        assert!(has_post_compaction_usage(&branch, &ids));
    }

    #[test]
    fn usage_keeps_tokens_and_percent_in_sync() {
        let known = usage(Some(1000), 200_000);
        assert_eq!(known.tokens, Some(1000));
        assert_eq!(known.percent, Some(0.5));
        let unknown = usage(None, 200_000);
        assert_eq!(unknown.tokens, None);
        assert_eq!(unknown.percent, None, "未知时百分比也必须是 None");
        // 窗口非法 ⇒ ref 返回 undefined，这里按未知处理。
        let zero_window = usage(Some(1000), 0);
        assert_eq!(zero_window.tokens, None);
        assert_eq!(zero_window.percent, None);
    }
}
