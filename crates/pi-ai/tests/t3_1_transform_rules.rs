//! P3-2：孤儿 tool call 与 error/aborted assistant 的处理规则。
//!
//! 依据 `ref/pi-0.87.1/packages/ai/src/api/transform-messages.ts:193-232`：
//! - assistant 的 `stopReason` 为 `error`/`aborted` 时**整条丢弃**；
//! - 未被 tool_result 闭合的 tool call，在遇到下一条 user 消息或对话结束时
//!   合成 `is_error: true` + "No result provided"。

use pi_ai::{Context, StopReason};

mod common;

fn msg(role: common::Role, content: Vec<common::Block>) -> common::ContextMessage {
    common::ContextMessage {
        role,
        content,
        stop_reason: None,
    }
}

fn assistant_with_stop(stop: StopReason, content: Vec<common::Block>) -> common::ContextMessage {
    common::ContextMessage {
        role: common::Role::Assistant,
        content,
        stop_reason: Some(stop),
    }
}

fn user(text: &str) -> common::ContextMessage {
    msg(
        common::Role::User,
        vec![common::Block::Text { text: text.into() }],
    )
}

fn tool_call(id: &str) -> common::Block {
    common::Block::ToolUse {
        id: id.to_string(),
        name: "read".to_string(),
        input: serde_json::json!({ "path": "a.txt" }),
    }
}

fn context(messages: Vec<common::ContextMessage>) -> Context {
    common::ctx_msgs(None, messages)
}

#[test]
fn aborted_assistant_message_is_dropped_entirely() {
    let context = context(vec![
        user("first"),
        assistant_with_stop(
            StopReason::Aborted,
            vec![common::Block::Text {
                text: "partial".into(),
            }],
        ),
        user("second"),
    ]);
    let messages = context.to_anthropic_messages();
    assert_eq!(messages.len(), 2, "aborted 的 assistant 整条丢弃");
    assert_eq!(messages[0]["content"][0]["text"], "first");
    assert_eq!(messages[1]["content"][0]["text"], "second");
}

#[test]
fn errored_assistant_message_is_dropped_entirely() {
    let context = context(vec![
        user("first"),
        assistant_with_stop(StopReason::Error, vec![tool_call("call_orphan")]),
        user("second"),
    ]);
    let messages = context.to_anthropic_messages();
    assert_eq!(messages.len(), 2, "error 的 assistant 整条丢弃");
    assert!(
        messages[1]["content"][0]["type"] == "text",
        "下一条 user 消息本身不该被替换：{}",
        messages[1]["content"][0]
    );
}

#[test]
fn orphaned_tool_call_gets_a_synthetic_error_result() {
    let context = context(vec![
        user("first"),
        msg(common::Role::Assistant, vec![tool_call("call_orphan")]),
        user("second"),
    ]);
    let messages = context.to_anthropic_messages();
    // TS 的顺序：user → assistant(tool_use) → 合成的 tool_result → user。
    assert_eq!(messages.len(), 4);
    let synthetic = &messages[2];
    assert_eq!(synthetic["role"], "user");
    let block = &synthetic["content"][0];
    assert_eq!(block["type"], "tool_result");
    assert_eq!(block["tool_use_id"], "call_orphan");
    assert_eq!(block["is_error"], true);
    assert_eq!(block["content"], "No result provided");
}

#[test]
fn closed_tool_calls_are_not_synthesized() {
    let context = context(vec![
        user("read it"),
        msg(common::Role::Assistant, vec![tool_call("call_1")]),
        msg(
            common::Role::User,
            vec![common::Block::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: vec![common::Block::Text {
                    text: "body".into(),
                }],
                is_error: false,
            }],
        ),
        user("done"),
    ]);
    let messages = context.to_anthropic_messages();
    let synthetic_count = messages
        .iter()
        .filter(|m| m["content"][0]["type"] == "tool_result" && m["content"][0]["is_error"] == true)
        .count();
    assert_eq!(synthetic_count, 0, "已闭合的 tool call 不该被合成补齐");
}

#[test]
fn trailing_orphaned_tool_call_is_synthesized_at_the_end() {
    let context = context(vec![
        user("read it"),
        msg(common::Role::Assistant, vec![tool_call("call_tail")]),
    ]);
    let messages = context.to_anthropic_messages();
    assert_eq!(messages.len(), 3, "对话结束时也要补齐");
    assert_eq!(messages[2]["content"][0]["tool_use_id"], "call_tail");
    assert_eq!(messages[2]["content"][0]["is_error"], true);
}

#[test]
fn dropped_assistant_closes_its_pending_tool_calls_first() {
    let context = context(vec![
        user("first"),
        msg(common::Role::Assistant, vec![tool_call("call_ok")]),
        assistant_with_stop(StopReason::Aborted, vec![tool_call("call_dropped")]),
        user("second"),
    ]);
    let messages = context.to_anthropic_messages();
    let ids: Vec<&str> = messages
        .iter()
        .filter(|m| m["content"][0]["type"] == "tool_result")
        .map(|m| m["content"][0]["tool_use_id"].as_str().unwrap_or_default())
        .collect();
    assert!(
        ids.contains(&"call_ok"),
        "前一条 assistant 的 tool call 应被补齐：{ids:?}"
    );
    assert!(
        !ids.contains(&"call_dropped"),
        "被丢弃的 assistant 不应留下 tool call：{ids:?}"
    );
}

#[test]
fn openai_conversion_applies_the_same_rules() {
    let context = context(vec![
        user("first"),
        assistant_with_stop(
            StopReason::Aborted,
            vec![common::Block::Text {
                text: "partial".into(),
            }],
        ),
        msg(common::Role::Assistant, vec![tool_call("call_orphan")]),
        user("second"),
    ]);
    let messages = context.to_openai_messages();
    let roles: Vec<&str> = messages
        .iter()
        .map(|m| m["role"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        roles,
        vec!["user", "assistant", "tool", "user"],
        "openai 侧同样丢弃 aborted 并为未闭合调用补齐：{roles:?}"
    );

    assert_eq!(
        messages[2]["content"], "No result provided",
        "补齐结果排在 user 之前"
    );
}
