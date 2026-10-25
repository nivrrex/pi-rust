//! P3.0：转换规则**作用于 `Message`**（原实现在 `common::Block` 上）。
//!
//! 规则取自 `transform-messages.ts:166-232`；这里验证与 `Context` 版本一致的三件事：
//! 丢弃 error/aborted 的 assistant、为未闭合的 tool call 合成结果、合成结果排在 user 之前。

use pi_ai::message::{transform_messages, Message, ToolResultContent, UserContent};
use pi_ai::{AssistantMessage, Content, Cost, StopReason, Usage};
use serde_json::Map;

fn assistant(stop: StopReason, content: Vec<Content>) -> Message {
    Message::Assistant(Box::new(AssistantMessage {
        role: "assistant".to_string(),
        content,
        api: "openai-completions".to_string(),
        provider: "mock".to_string(),
        model: "mock-model".to_string(),
        response_model: None,
        usage: Usage {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: None,
            reasoning: None,
            total_tokens: 0,
            cost: Cost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
                total: 0.0,
            },
        },
        stop_reason: stop,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 2,
    }))
}

fn user(text: &str, timestamp: i64) -> Message {
    Message::User {
        content: UserContent::Text(text.to_string()),
        timestamp,
        extra: Map::new(),
    }
}

fn tool_call(id: &str) -> Content {
    Content::ToolCall {
        id: id.to_string(),
        name: "read".to_string(),
        arguments: Map::new(),
        thought_signature: None,
    }
}

fn text(t: &str) -> Content {
    Content::Text {
        text: t.to_string(),
        text_signature: None,
    }
}

#[test]
fn aborted_and_errored_assistants_are_dropped() {
    for stop in [StopReason::Aborted, StopReason::Error] {
        let messages = vec![
            user("first", 1),
            assistant(stop, vec![text("partial")]),
            user("second", 3),
        ];
        let out = transform_messages(&messages);
        assert_eq!(out.len(), 2, "{stop:?} 的 assistant 应被丢弃");
        assert_eq!(out[0].role(), "user");
        assert_eq!(out[1].role(), "user");
    }
}

#[test]
fn normal_assistant_is_kept() {
    let messages = vec![
        user("first", 1),
        assistant(StopReason::Stop, vec![text("ok")]),
    ];
    let out = transform_messages(&messages);
    assert_eq!(out.len(), 2);
    assert_eq!(out[1].role(), "assistant");
}

#[test]
fn orphaned_tool_call_gets_a_synthetic_error_result() {
    let messages = vec![
        user("read it", 1),
        assistant(StopReason::ToolUse, vec![tool_call("call_orphan")]),
        user("next", 3),
    ];
    let out = transform_messages(&messages);
    assert_eq!(
        out.len(),
        4,
        "顺序应是 user → assistant → 合成 toolResult → user"
    );
    assert_eq!(out[2].role(), "toolResult");
    match &out[2] {
        Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            is_error,
            ..
        } => {
            assert_eq!(tool_call_id, "call_orphan");
            assert_eq!(tool_name, "read");
            assert!(*is_error);
            assert_eq!(
                content,
                &vec![ToolResultContent::Text {
                    text: "No result provided".to_string(),
                    extra: Map::new()
                }]
            );
        }
        other => panic!("应为 toolResult，实际 {}", other.role()),
    }
}

#[test]
fn answered_tool_calls_are_not_synthesized() {
    let messages = vec![
        user("read it", 1),
        assistant(StopReason::ToolUse, vec![tool_call("call_1")]),
        Message::ToolResult {
            tool_call_id: "call_1".to_string(),
            tool_name: "read".to_string(),
            content: vec![ToolResultContent::Text {
                text: "body".to_string(),
                extra: Map::new(),
            }],
            is_error: false,
            timestamp: 3,
            extra: Map::new(),
        },
        user("done", 4),
    ];
    let out = transform_messages(&messages);
    assert_eq!(out.len(), 4, "已应答的调用不该被补齐");
    assert!(
        out.iter().all(|m| m.role() != "toolResult"
            || !matches!(m, Message::ToolResult { is_error: true, .. })),
        "不应出现 is_error 的合成结果"
    );
}

#[test]
fn trailing_orphaned_tool_call_is_synthesized_at_the_end() {
    let messages = vec![
        user("read it", 1),
        assistant(StopReason::ToolUse, vec![tool_call("call_tail")]),
    ];
    let out = transform_messages(&messages);
    assert_eq!(out.len(), 3, "对话结束时也要补齐");
    assert_eq!(out[2].role(), "toolResult");
}

#[test]
fn dropped_assistant_closes_its_pending_tool_calls_first() {
    let messages = vec![
        user("first", 1),
        assistant(StopReason::ToolUse, vec![tool_call("call_ok")]),
        assistant(StopReason::Aborted, vec![tool_call("call_dropped")]),
        user("second", 4),
    ];
    let out = transform_messages(&messages);
    let ids: Vec<String> = out
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect();
    assert!(
        ids.contains(&"call_ok".to_string()),
        "前一条的调用应被补齐：{ids:?}"
    );
    assert!(
        !ids.contains(&"call_dropped".to_string()),
        "被丢弃的 assistant 不该留下调用：{ids:?}"
    );
}
