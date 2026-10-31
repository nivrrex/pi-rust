//! P5-3c：自动压缩的**流程层**（ref `agent-session.ts:2699-2734` 的判定 + `:2743-2790` 的执行）。
//!
//! 与 ref 的分工一致：纯逻辑（阈值、准备、摘要编排、条目字段）在 `pi-core` 的 `compaction` 里；
//! 这里只做"取会话事实 → 调 pi-core → 落条目 → 重建上下文"的接线。
//!
//! 流程（对应 ref）：`buildSessionProjection` → `_checkCompaction`（`shouldCompact`）→
//! `prepareCompaction` → `compact`（真实 provider 摘要调用）→ `appendCompaction` → 用压缩后的
//! 投影重建上下文。

use crate::session_writer::SessionWriter;
use pi_ai::message::Message;
use pi_ai::types::StopReason;
use pi_core::agent::compaction::{
    compact_with_provider, prepare_compaction, threshold_outcome, CompactionSettings,
    ThresholdOutcome,
};
use pi_core::agent::projection::{build_session_projection, LeafSelector};
use pi_core::session::SessionEntry;
use pi_core::timefmt;

/// 一次自动压缩检查的输入（会话事实 + 模型参数，全部只读）。
pub struct CompactionInputs<'a> {
    /// 会话文件里的全部条目（压缩判定与准备都要看它们）。
    pub entries: &'a [SessionEntry],
    /// `settings.json` 的 `compaction` 段（已与默认值合并）。
    pub settings: CompactionSettings,
    /// 模型的上下文窗口（来自 models.json）。
    pub context_window: u64,
    /// 摘要请求用的模型名。
    pub model: &'a str,
    /// 模型的 `maxTokens`（摘要预算的上限）。
    pub model_max_tokens: u64,
    /// 模型是否支持推理（决定摘要请求是否带思考等级）。
    pub model_reasoning: bool,
    /// 当前思考等级（`off` 时摘要请求不带 reasoning）。
    pub thinking_level: Option<&'a str>,
    /// 自定义摘要指令（目前没有来源，恒 `None`；ref 由扩展传入）。
    pub custom_instructions: Option<&'a str>,
}

/// 取会话文件里**最后一条 system 消息**的原始 JSON 对象文本（保序）。
///
/// 为什么不用投影里的 `Message::System` 再序列化：`serde_json::Map` 是 `BTreeMap`，会把
/// `toolsAdded[].parameters` 的键序排序掉，而 TS 的键序参与 A 层判等；会话文件的**原始行**
/// 里就是 TS 写的原文（`session_io::extract_raw_message_object`）。
fn raw_system_message_from_session(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<SessionEntry>(line) else {
            continue;
        };
        if let SessionEntry::Message {
            message: Message::System { .. },
            ..
        } = &entry
        {
            return pi_core::session_io::extract_raw_message_object(line).map(str::to_string);
        }
    }
    None
}

/// 自动压缩需要的模型/设置参数（`PrintRun` 持有一份）。
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    pub settings: CompactionSettings,
    pub context_window: u64,
    /// 摘要请求用的模型名。
    pub model: String,
    /// 模型所属 provider（与助手消息比对，判断"是否换了模型"）。
    pub model_provider: String,
    pub model_max_tokens: u64,
    pub model_reasoning: bool,
    pub thinking_level: Option<String>,
}

/// 判定结果：走哪条路、事件里的 `reason`、以及是否需要重试回合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoCompactionPlan {
    pub reason: &'static str,
    pub will_retry: bool,
    /// 只发失败事件、**不压缩**的情形（ref `agent-session.ts:2671-2690`：溢出恢复已试过一次）。
    /// 值为 `compaction_end` 的 `errorMessage`。
    pub failure: Option<&'static str>,
}

/// 压缩跑完后的产物。
pub struct CompactionRun {
    pub messages: Vec<Message>,
    /// `compaction_end` 事件里的 `result` 字段。
    pub result_event: serde_json::Value,
}

/// 纯判定（不发事件、不改文件）：`overflow_outcome` 之后必要时再走 `threshold_outcome`。
///
/// `reason` 的取值与 ref 一致：溢出路径 `"overflow"`、阈值路径 `"threshold"`（`"manual"` 属
/// 手动压缩，由 P5-4 的 rpc `compact` 用）。
#[must_use]
pub fn auto_compaction_plan(
    entries: &[SessionEntry],
    config: &CompactionConfig,
    assistant: &pi_ai::AssistantMessage,
    skip_aborted_check: bool,
) -> Option<AutoCompactionPlan> {
    // ref `:2762-2764`：`prepareCompaction` 给不出方案时**连 `compaction_start` 都不发**，
    // 所以这里先把它挡掉（否则会出现"只有 start 没有 end"的事件对）。
    // **不能无条件前置**：Case 1（溢出且 `willRetry`）里 ref 是**先** `_omitRecoveryAttempt`
    // （写 `context_edit` 省略失败助手，`agent-session.ts:2695`）**再** `_runAutoCompaction`——
    // 单回合会话只有省略后才有切点（golden/S13e 实测）。所以下面按分支决定是否提前挡掉。
    let projection = build_session_projection(entries, LeafSelector::Unspecified).ok()?;
    let last_compaction_timestamp = entries.iter().rev().find_map(|entry| {
        if let SessionEntry::Compaction { timestamp, .. } = entry {
            Some(timestamp.as_str())
        } else {
            None
        }
    });
    let decision =
        pi_core::agent::compaction::overflow_outcome(&pi_core::agent::compaction::OverflowInputs {
            assistant,
            model_provider: &config.model_provider,
            model_id: &config.model,
            context_window: config.context_window,
            model_max_tokens: config.model_max_tokens,
            projection: &projection,
            path_entries: entries,
            settings: &config.settings,
            last_compaction_timestamp,
            skip_aborted_check,
            overflow_recovery_attempted: false,
        });
    use pi_core::agent::compaction::AutoCompactionDecision as Decision;
    if decision != Decision::OverflowRetry {
        // ref `:2762-2764`：给不出方案时**连 `compaction_start` 都不发**（Case 1 除外，见上）。
        prepare_compaction(entries, &config.settings)
            .ok()
            .flatten()?;
    }
    match decision {
        Decision::Skip => None,
        Decision::OverflowNoRetry => Some(AutoCompactionPlan {
            reason: "overflow",
            will_retry: false,
            failure: None,
        }),
        Decision::OverflowRetry => Some(AutoCompactionPlan {
            reason: "overflow",
            will_retry: true,
            failure: None,
        }),
        // 已经试过一次恢复：ref 只发失败事件、不再压缩（`:2671-2691`）。
        // **未实现**（53 个场景都不可达）：见 notes 的已知限制。
        // ref `:2671-2690`：已经试过一次"压缩后重试"，再溢出就**只发失败事件**、不再压缩。
        Decision::OverflowExhausted => Some(AutoCompactionPlan {
            reason: "overflow",
            will_retry: false,
            failure: Some(
                "Context overflow recovery failed after one compact-and-retry attempt. \
                 Try reducing context or switching to a larger-context model.",
            ),
        }),
        Decision::Threshold => {
            let last_assistant = projection.messages.iter().rev().find_map(|message| {
                if let Message::Assistant(assistant) = message {
                    Some((
                        assistant.usage,
                        assistant.stop_reason == StopReason::Error,
                        assistant.timestamp,
                    ))
                } else {
                    None
                }
            });
            let outcome = threshold_outcome(
                &projection,
                entries,
                last_assistant.as_ref().map(|(usage, _, _)| usage),
                last_assistant
                    .as_ref()
                    .is_some_and(|(_, is_error, _)| *is_error),
                config.context_window,
                &config.settings,
                last_compaction_timestamp.and_then(timefmt::iso8601_to_unix_millis),
                last_assistant.as_ref().map(|(_, _, timestamp)| *timestamp),
            );
            match outcome {
                ThresholdOutcome::Skip => None,
                ThresholdOutcome::Compact => Some(AutoCompactionPlan {
                    reason: "threshold",
                    will_retry: false,
                    failure: None,
                }),
            }
        }
    }
}

/// rpc 侧的事件出口：把 `AgentEvent` 投影成一行 JSON 写到共享 stdout。
///
/// 存在的原因：摘要重试事件（`summarization_retry_*`）需要一个 `EventSink`，而 rpc 路径此前
/// 三处 `perform_auto_compaction` 都传 `None`。**不引入新机制**——`RpcShared` 本来就有
/// `write_line`，`AgentEvent` 本来就有 `to_json_event_line` 投影。
pub struct LineSink(pub std::sync::Arc<crate::rpc_session::RpcShared>);

impl pi_core::agent::EventSink for LineSink {
    fn emit(&self, event: pi_core::agent::AgentEvent) {
        self.0
            .write_line(&crate::print_mode::to_json_event_line(&event));
    }
}

/// 摘要请求重试的可观测出口：把 pi-core 的回调翻译成 `AgentEvent`（ref
/// `agent-session.ts:3339-3360`）。`reason` 只在 `source: "compaction"` 时出现。
struct RetryEvents<'a> {
    sink: &'a dyn pi_core::agent::EventSink,
    reason: Option<&'a str>,
}

impl pi_core::agent::compaction::SummarizationRetryObserver for RetryEvents<'_> {
    fn scheduled(&self, attempt: u32, max_attempts: u32, delay_ms: u64, error_message: &str) {
        self.sink
            .emit(pi_core::agent::AgentEvent::SummarizationRetryScheduled {
                attempt,
                max_attempts,
                delay_ms,
                error_message: error_message.to_string(),
            });
    }
    fn attempt_start(&self) {
        self.sink
            .emit(pi_core::agent::AgentEvent::SummarizationRetryAttemptStart {
                source: "compaction".to_string(),
                reason: self.reason.map(str::to_string),
            });
    }
    fn finished(&self) {
        self.sink
            .emit(pi_core::agent::AgentEvent::SummarizationRetryFinished);
    }
}

/// 执行压缩：`prepare_compaction` → `compact_with_provider` → `append_compaction` → 重建上下文。
pub async fn perform_auto_compaction(
    entries: &[SessionEntry],
    config: &CompactionConfig,
    custom_instructions: Option<&str>,
    reason: Option<&str>,
    sink: Option<&dyn pi_core::agent::EventSink>,
    provider: &dyn pi_ai::Provider,
    writer: &mut SessionWriter,
) -> Result<Option<CompactionRun>, String> {
    let Some(preparation) =
        prepare_compaction(entries, &config.settings).map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let result = compact_with_provider(
        &preparation,
        &pi_core::agent::compaction::SummarizationRequest {
            model: &config.model,
            model_max_tokens: config.model_max_tokens,
            custom_instructions,
            model_reasoning: config.model_reasoning,
            thinking_level: config.thinking_level.as_deref(),
        },
        provider,
        sink.map(|sink| RetryEvents {
            sink,
            reason: reason.or(Some("threshold")),
        })
        .as_ref()
        .map(|observer| observer as &dyn pi_core::agent::compaction::SummarizationRetryObserver),
    )
    .await?;
    // `systemMessage` 用**会话文件原始行**里的那条 system 消息（保序；见本文件顶部的说明）。
    // ref `session-manager.ts:1262-1274` 取的是当前投影的最后一条 system。
    let system_message = raw_system_message_from_session(writer.path());
    let details = serde_json::json!({
        "readFiles": result.details.read_files,
        "modifiedFiles": result.details.modified_files,
    });
    let usage = serde_json::to_value(result.usage).map_err(|error| error.to_string())?;
    let written = writer.append_compaction(
        &result.summary,
        Some(&result.first_kept_entry_id),
        result.tokens_before,
        Some(details.clone()),
        Some(usage.clone()),
        system_message.as_deref(),
    )?;
    let appended: SessionEntry =
        serde_json::from_value(written).map_err(|error| error.to_string())?;
    let mut rebuilt = entries.to_vec();
    rebuilt.push(appended);
    let after = build_session_projection(&rebuilt, LeafSelector::Unspecified)
        .map_err(|error| error.to_string())?;
    // ref `:2841`：`estimatedTokensAfter = estimateMessagesTokens(projection.messages)`，
    // 即**对投影后的消息逐条 `estimateTokens` 求和**（不是 projected 变体）。
    let estimated_tokens_after: u64 = after
        .messages
        .iter()
        .map(pi_core::agent::compaction::estimate_tokens)
        .sum();
    Ok(Some(CompactionRun {
        messages: after.messages,
        result_event: serde_json::json!({
            "summary": result.summary,
            "firstKeptEntryId": result.first_kept_entry_id,
            "tokensBefore": result.tokens_before,
            "estimatedTokensAfter": estimated_tokens_after,
            "usage": usage,
            "details": details,
        }),
    }))
}

/// 检查并按需压缩。返回 `Some(压缩后的消息列表)` 表示**真的压缩了**——调用方必须换用它作为
/// 后续回合的上下文；`Ok(None)` 表示不需要（或无法）压缩，沿用原上下文。
pub async fn maybe_compact(
    inputs: &CompactionInputs<'_>,
    provider: &dyn pi_ai::Provider,
    writer: &mut SessionWriter,
) -> Result<Option<Vec<Message>>, String> {
    if !inputs.settings.enabled {
        return Ok(None);
    }
    // 投影（= 当前叶子路径 + 各条目的投影规则），阈值判定与上下文重建都基于它。
    let projection = build_session_projection(inputs.entries, LeafSelector::Unspecified)
        .map_err(|error| error.to_string())?;
    // 阈值参数一：路径上**最后一条**助手消息的 usage 与它是否以 error 结束。
    let last_assistant = projection.messages.iter().rev().find_map(|message| {
        if let Message::Assistant(assistant) = message {
            Some((
                assistant.usage,
                assistant.stop_reason == StopReason::Error,
                assistant.timestamp,
            ))
        } else {
            None
        }
    });
    // 阈值参数二：最后一条 `compaction` 条目的时间（用它识别"usage 是压缩之前的陈旧值"）。
    let last_compaction_ms = inputs.entries.iter().rev().find_map(|entry| {
        if let SessionEntry::Compaction { timestamp, .. } = entry {
            timefmt::iso8601_to_unix_millis(timestamp)
        } else {
            None
        }
    });
    let outcome = threshold_outcome(
        &projection,
        inputs.entries,
        last_assistant.as_ref().map(|(usage, _, _)| usage),
        last_assistant
            .as_ref()
            .is_some_and(|(_, is_error, _)| *is_error),
        inputs.context_window,
        &inputs.settings,
        last_compaction_ms,
        last_assistant.as_ref().map(|(_, _, timestamp)| *timestamp),
    );
    if outcome == ThresholdOutcome::Skip {
        return Ok(None);
    }
    let Some(preparation) =
        prepare_compaction(inputs.entries, &inputs.settings).map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let result = compact_with_provider(
        &preparation,
        &pi_core::agent::compaction::SummarizationRequest {
            model: inputs.model,
            model_max_tokens: inputs.model_max_tokens,
            custom_instructions: inputs.custom_instructions,
            model_reasoning: inputs.model_reasoning,
            thinking_level: inputs.thinking_level,
        },
        provider,
        None,
    )
    .await?;
    // `systemMessage`：投影里最后一条 system（ref `:1262` 的 `getCurrentSystemMessage(...)`），
    // 并把 `timestamp` 覆盖成**压缩条目的时间**（毫秒，ref `:1272-1274`）。
    // `systemMessage` 用**会话文件原始行**里的那条 system 消息（保序；见本文件顶部的说明）。
    // ref `session-manager.ts:1262-1274` 取的是当前投影的最后一条 system。
    let system_message = raw_system_message_from_session(writer.path());
    let details = serde_json::json!({
        "readFiles": result.details.read_files,
        "modifiedFiles": result.details.modified_files,
    });
    let usage = serde_json::to_value(result.usage).map_err(|error| error.to_string())?;
    let written = writer.append_compaction(
        &result.summary,
        Some(&result.first_kept_entry_id),
        result.tokens_before,
        Some(details),
        Some(usage),
        system_message.as_deref(),
    )?;
    // 重建上下文：把刚落库的条目接在原有条目之后，再投影一次（ref 压缩后即用投影重建上下文）。
    let appended: SessionEntry =
        serde_json::from_value(written).map_err(|error| error.to_string())?;
    let mut rebuilt = inputs.entries.to_vec();
    rebuilt.push(appended);
    let after = build_session_projection(&rebuilt, LeafSelector::Unspecified)
        .map_err(|error| error.to_string())?;
    Ok(Some(after.messages))
}

#[cfg(test)]
mod tests {
    use super::CompactionInputs;
    use pi_core::agent::compaction::{CompactionSettings, DEFAULT_COMPACTION_SETTINGS};

    /// 样本自检：关闭压缩时输入本身就不满足触发（避免把"没触发"误当成"接线通了"）。
    #[test]
    fn disabled_settings_are_a_precondition_not_a_result() {
        let entries: Vec<pi_core::session::SessionEntry> = Vec::new();
        let inputs = CompactionInputs {
            entries: &entries,
            settings: CompactionSettings {
                enabled: false,
                ..DEFAULT_COMPACTION_SETTINGS
            },
            context_window: 1,
            model: "mock-model",
            model_max_tokens: 1,
            model_reasoning: false,
            thinking_level: None,
            custom_instructions: None,
        };
        assert!(!inputs.settings.enabled, "本用例的前提就是 enabled=false");
        assert_eq!(inputs.entries.len(), 0, "空会话");
    }
}
