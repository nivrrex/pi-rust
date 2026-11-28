//! 工具名必须"首值优先"，且坏数据不得毒化请求（F243）。
//!
//! 真机链条：网关在后续 chunk 里补发 `"function": {"name": ""}`，pi-rs 无条件覆盖，
//! 把已经拿到的名字擦成空 ⇒ 执行时报 `Tool  not found`，并把 `"name": ""` 写进会话 ⇒
//! 之后每次请求都被拒 `400 [input_invalid] tool_calls.function.name is required`。
//!
//! ref `openai-completions.ts:539-541` 是 `if (!block.name && name) block.name = name;`，
//! 即**只在还没有名字时取第一个非空值**，后来者不覆盖。

use pi_ai::context::Context;
use pi_ai::decoder::StreamDecoder;
use pi_ai::message::{Content, Message};
use pi_ai::types::AssistantMessage;
use pi_ai::Rates;

fn sse(chunks: &[&str]) -> String {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str("data: ");
        body.push_str(chunk);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn decode(body: &str) -> AssistantMessage {
    let rates = Rates {
        input: 0.0,
        output: 0.0,
        cache_read: 0.0,
        cache_write: 0.0,
    };
    let mut decoder = StreamDecoder::new("openai-completions", "mock", "mock-model", rates)
        .expect("解码器可创建");
    for chunk in body.split("\n\n") {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let payload = format!("{chunk}\n\n");
        let _ = decoder.push(payload.as_bytes());
    }
    let _ = decoder.finish();
    decoder.final_message()
}

/// 名字先到、后来补发空名 ⇒ 必须保留第一个名字。
#[test]
fn later_empty_name_does_not_erase_the_first() {
    let body = sse(&[
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"bash","arguments":"{\"cmd\":"}}]},"finish_reason":null}]}"#,
        // 网关补发的空名分片
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"","arguments":"\"echo hi\"}"}}]},"finish_reason":null}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);
    let message = decode(&body);
    let names: Vec<&str> = message
        .content
        .iter()
        .filter_map(|block| match block {
            Content::ToolCall { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["bash"], "空名分片不得覆盖已有工具名：{names:?}");
}

/// 名字比 id 晚到也要能拿到（首值优先不等于只在第一个 chunk 取）。
#[test]
fn name_arriving_after_the_id_chunk_is_kept() {
    let body = sse(&[
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_b","function":{"arguments":"{}"}}]},"finish_reason":null}]}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"read"}}]},"finish_reason":null}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);
    let message = decode(&body);
    let names: Vec<&str> = message
        .content
        .iter()
        .filter_map(|block| match block {
            Content::ToolCall { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["read"], "后到的名字要补上：{names:?}");
}

/// **已被写进会话**的空名调用：序列化请求时连同它的 tool 结果一起剔除，
/// 不能把 `"name": ""` 再发出去。
#[test]
fn empty_name_calls_are_dropped_from_the_request() {
    let mut args_bad = serde_json::Map::new();
    args_bad.insert("cmd".to_string(), serde_json::json!("echo hi"));
    let mut args_ok = serde_json::Map::new();
    args_ok.insert("cmd".to_string(), serde_json::json!("echo ok"));

    let assistant = AssistantMessage {
        role: "assistant".to_string(),
        content: vec![
            Content::Text {
                text: "calling".to_string(),
                text_signature: None,
            },
            Content::ToolCall {
                id: "call_bad".to_string(),
                name: String::new(),
                arguments: args_bad,
                thought_signature: None,
            },
            Content::ToolCall {
                id: "call_ok".to_string(),
                name: "bash".to_string(),
                arguments: args_ok,
                thought_signature: None,
            },
        ],
        api: "openai-completions".to_string(),
        provider: "acme".to_string(),
        model: "mock".to_string(),
        response_model: None,
        usage: Default::default(),
        stop_reason: pi_ai::types::StopReason::ToolUse,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 0,
    };

    let context = Context {
        system: Some("sys".to_string()),
        messages: vec![
            Message::User {
                content: pi_ai::message::UserContent::Text("go".to_string()),
                timestamp: 0,
                extra: serde_json::Map::new(),
            },
            Message::Assistant(Box::new(assistant)),
            Message::ToolResult {
                tool_call_id: "call_bad".to_string(),
                tool_name: String::new(),
                content: vec![pi_ai::message::ToolResultContent::Text {
                    text: "Tool  not found".to_string(),
                    extra: serde_json::Map::new(),
                }],
                is_error: true,
                timestamp: 0,
                extra: serde_json::Map::new(),
            },
            Message::ToolResult {
                tool_call_id: "call_ok".to_string(),
                tool_name: "bash".to_string(),
                content: vec![pi_ai::message::ToolResultContent::Text {
                    text: "ok".to_string(),
                    extra: serde_json::Map::new(),
                }],
                is_error: false,
                timestamp: 0,
                extra: serde_json::Map::new(),
            },
        ],
        ..Default::default()
    };
    let body = context.to_openai_messages_with_provider("acme");
    let text = serde_json::to_string(&body).expect("序列化");
    assert!(
        !text.contains("\"name\":\"\""),
        "请求体里不得出现空工具名：{text:.600}"
    );
    assert!(
        !text.contains("call_bad"),
        "空名调用及其结果都应被剔除：{text:.600}"
    );
    assert!(
        text.contains("\"name\":\"bash\"") && text.contains("call_ok"),
        "正常的工具调用必须保留：{text:.600}"
    );
}
