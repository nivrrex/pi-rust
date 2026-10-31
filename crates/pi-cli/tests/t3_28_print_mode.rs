//! P3.5：print 模式的输出与退出码（纯函数部分）。

use pi_ai::message::Message;
use pi_ai::{AssistantMessage, Content, StopReason, Usage};
use pi_cli::print_mode::{exit_code_for, text_output};

fn assistant(stop: StopReason, content: Vec<Content>, error: Option<&str>) -> Message {
    Message::Assistant(Box::new(AssistantMessage {
        role: "assistant".to_string(),
        content,
        api: "openai-completions".to_string(),
        provider: "mock".to_string(),
        model: "m".to_string(),
        response_model: None,
        usage: Usage::default(),
        stop_reason: stop,
        error_message: error.map(str::to_string),
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 0,
    }))
}

fn text(value: &str) -> Content {
    Content::Text {
        text: value.to_string(),
        text_signature: None,
    }
}

fn user(value: &str) -> Message {
    Message::User {
        content: pi_ai::UserContent::Text(value.to_string()),
        timestamp: 0,
        extra: serde_json::Map::new(),
    }
}

#[test]
fn text_output_takes_only_text_blocks_and_appends_a_newline_each() {
    let messages = vec![
        user("hi"),
        assistant(StopReason::Stop, vec![text("one"), text("two")], None),
    ];
    assert_eq!(text_output(&messages), "one\ntwo\n");
}

#[test]
fn text_output_ignores_thinking_and_tool_calls() {
    let messages = vec![assistant(
        StopReason::ToolUse,
        vec![
            Content::Thinking {
                thinking: "hmm".to_string(),
                thinking_signature: None,
                redacted: None,
            },
            text("answer"),
            Content::ToolCall {
                id: "c".to_string(),
                name: "read".to_string(),
                arguments: serde_json::Map::new(),
                thought_signature: None,
            },
        ],
        None,
    )];
    assert_eq!(text_output(&messages), "answer\n");
}

#[test]
fn normal_stop_has_no_error_exit() {
    let messages = vec![assistant(StopReason::Stop, vec![text("ok")], None)];
    assert_eq!(exit_code_for(&messages), (0, Vec::new()));
}

#[test]
fn error_and_aborted_stops_exit_one_with_their_message() {
    let error = vec![assistant(StopReason::Error, vec![], Some("boom"))];
    assert_eq!(exit_code_for(&error), (1, vec!["boom".to_string()]));

    let aborted = vec![assistant(StopReason::Aborted, vec![], Some("stopped"))];
    assert_eq!(exit_code_for(&aborted), (1, vec!["stopped".to_string()]));
}

#[test]
fn missing_error_message_falls_back_to_a_request_label() {
    let error = vec![assistant(StopReason::Error, vec![], None)];
    assert_eq!(
        exit_code_for(&error),
        (1, vec!["Request error".to_string()])
    );
    let aborted = vec![assistant(StopReason::Aborted, vec![], None)];
    assert_eq!(
        exit_code_for(&aborted),
        (1, vec!["Request aborted".to_string()])
    );
}

#[test]
fn only_the_last_message_decides_the_exit_code() {
    // 最后一条不是 assistant（例如以 toolResult 结束）：不置错误码。
    let messages = vec![
        assistant(StopReason::Error, vec![], Some("earlier")),
        user("later"),
    ];
    assert_eq!(exit_code_for(&messages), (0, Vec::new()));
}

// —— json 投影（`spec/b-agent-loop.md` 第 3 节）——

use pi_cli::print_mode::{to_json_event, JsonEventError};
use pi_core::agent::AgentEvent;

fn assistant_message(content: Vec<Content>) -> Message {
    assistant(StopReason::Stop, content, None)
}

#[test]
fn message_update_is_rewritten_with_cumulative_usage_and_no_partial() {
    let event = AgentEvent::MessageUpdate {
        message: assistant_message(vec![text("hi")]),
        assistant_event: Some(pi_ai::AssistantEvent::TextDelta {
            content_index: 0,
            delta: "hi".to_string(),
        }),
    };
    let value = to_json_event(&event).expect("可投影");
    assert_eq!(value["type"], "message_update");
    assert!(value.get("partial").is_none(), "partial 必须被剥掉");
    assert!(value["usage"].is_object(), "usage 取累积用量");
    assert_eq!(value["assistantMessageEvent"]["type"], "text_delta");
    assert_eq!(value["assistantMessageEvent"]["contentIndex"], 0);
    assert_eq!(value["assistantMessageEvent"]["delta"], "hi");
}

#[test]
fn toolcall_start_carries_id_and_tool_name() {
    let event = AgentEvent::MessageUpdate {
        message: assistant_message(vec![Content::ToolCall {
            id: "call_1".to_string(),
            name: "read".to_string(),
            arguments: serde_json::Map::new(),
            thought_signature: None,
        }]),
        assistant_event: Some(pi_ai::AssistantEvent::ToolCallStart { content_index: 0 }),
    };
    let value = to_json_event(&event).expect("可投影");
    assert_eq!(value["assistantMessageEvent"]["id"], "call_1");
    assert_eq!(value["assistantMessageEvent"]["toolName"], "read");
}

#[test]
fn toolcall_start_on_a_non_tool_block_is_an_error() {
    let event = AgentEvent::MessageUpdate {
        message: assistant_message(vec![text("not a call")]),
        assistant_event: Some(pi_ai::AssistantEvent::ToolCallStart { content_index: 0 }),
    };
    assert_eq!(
        to_json_event(&event),
        Err(JsonEventError::NotAToolCall { index: 0 })
    );
}

#[test]
fn message_update_on_a_non_assistant_message_is_an_error() {
    let event = AgentEvent::MessageUpdate {
        message: Message::User {
            content: pi_ai::UserContent::Text("hi".to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        },
        assistant_event: None,
    };
    assert_eq!(
        to_json_event(&event),
        Err(JsonEventError::NotAnAssistantMessage)
    );
}

#[test]
fn other_events_keep_their_type_field() {
    let value = to_json_event(&AgentEvent::TurnStart).expect("可投影");
    assert_eq!(value["type"], "turn_start");
}
