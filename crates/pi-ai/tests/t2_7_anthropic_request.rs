//! T2.7：Anthropic Messages 请求体。
//!
//! 判据：`golden/S12-anthropic` 与 `golden/S25-anthropic` 的 requests.jsonl。
//! 字段形状照 `ref/pi-0.87.1/packages/ai/src/api/anthropic-messages.ts`。

use pi_ai::anthropic::{
    build_messages_request, build_messages_request_with_thinking, ThinkingConfig, ToolSpecAnthropic,
};
use pi_ai::ProviderRequest;

/// 单条 user 文本的上下文。
fn ctx_of(prompt: &str, system: Option<&str>) -> pi_ai::Context {
    pi_ai::Context::from_messages(
        system.map(str::to_string),
        &[pi_ai::Message::User {
            content: pi_ai::UserContent::Text(prompt.to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        }],
    )
}

fn request() -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: ctx_of(
            "think then answer",
            Some("You are an expert coding assistant."),
        ),
        max_completion_tokens: 4096,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    }
}

#[test]
fn request_body_has_the_recorded_shape() {
    let body = build_messages_request(&request(), &[]);
    let mut keys: Vec<&str> = body
        .as_object()
        .expect("请求体是对象")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["max_tokens", "messages", "model", "stream", "system"],
        "无工具无 thinking 时的字段集合"
    );
    assert_eq!(body["model"], "mock-model");
    assert_eq!(body["stream"], true);
    assert_eq!(body["max_tokens"], 4096);
}

#[test]
fn system_is_a_text_block_array() {
    let body = build_messages_request(&request(), &[]);
    let system = body["system"].as_array().expect("system 是数组");
    assert_eq!(system.len(), 1);
    assert_eq!(system[0]["type"], "text");
    assert_eq!(system[0]["text"], "You are an expert coding assistant.");
}

#[test]
fn user_message_carries_cache_control_ephemeral() {
    let body = build_messages_request(&request(), &[]);
    let messages = body["messages"].as_array().expect("messages 是数组");
    assert_eq!(messages[0]["role"], "user");
    let content = messages[0]["content"].as_array().expect("content 是数组");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "think then answer");
    assert_eq!(
        content[0]["cache_control"]["type"], "ephemeral",
        "golden 里 user 文本块带 cache_control"
    );
}

#[test]
fn system_is_omitted_when_absent() {
    let request = ProviderRequest {
        context: ctx_of("think then answer", None),
        ..request()
    };
    let body = build_messages_request(&request, &[]);
    assert!(body.get("system").is_none(), "没有 system 就不出现该字段");
}

#[test]
fn tools_use_name_description_and_input_schema() {
    let tools = vec![ToolSpecAnthropic {
        name: "read".to_string(),
        description: "Read the contents of a file.".to_string(),
        input_schema: serde_json::json!({"type": "object", "required": ["path"]}),
        eager_input_streaming: false,
    }];
    let body = build_messages_request(&request(), &tools);
    let sent = body["tools"].as_array().expect("tools 是数组");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["name"], "read");
    assert_eq!(sent[0]["description"], "Read the contents of a file.");
    assert_eq!(sent[0]["input_schema"]["required"][0], "path");
    assert_eq!(sent[0]["eager_input_streaming"], false);
}

/// thinking 块形状照 `golden/S25-anthropic`：`{type:"enabled", budget_tokens, display}`。
#[test]
fn thinking_block_matches_the_recorded_shape() {
    let thinking = ThinkingConfig::Enabled {
        budget_tokens: Some(3072),
        display: Some("summarized"),
    };
    let body = build_messages_request_with_thinking(&request(), &[], Some(thinking));
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 3072);
    assert_eq!(body["thinking"]["display"], "summarized");
}

/// TS 的缺省值（`api/anthropic-messages.ts:1175`）：budget 缺省 1024、display 缺省 summarized。
#[test]
fn thinking_enabled_uses_typescript_defaults() {
    let body = build_messages_request_with_thinking(
        &request(),
        &[],
        Some(ThinkingConfig::Enabled {
            budget_tokens: None,
            display: None,
        }),
    );
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(
        body["thinking"]["budget_tokens"], 1024,
        "缺省 budget 是 1024"
    );
    assert_eq!(
        body["thinking"]["display"], "summarized",
        "缺省 display 是 summarized"
    );
}

#[test]
fn adaptive_thinking_carries_effort_only_when_given() {
    let body = build_messages_request_with_thinking(
        &request(),
        &[],
        Some(ThinkingConfig::Adaptive {
            display: None,
            effort: Some("high"),
        }),
    );
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["thinking"]["output_config"]["effort"], "high");

    let without = build_messages_request_with_thinking(
        &request(),
        &[],
        Some(ThinkingConfig::Adaptive {
            display: None,
            effort: None,
        }),
    );
    assert!(
        without["thinking"].get("output_config").is_none(),
        "没给 effort 就不发 output_config"
    );
}

#[test]
fn thinking_disabled_shape() {
    let body =
        build_messages_request_with_thinking(&request(), &[], Some(ThinkingConfig::Disabled));
    assert_eq!(body["thinking"]["type"], "disabled");
}

#[test]
fn thinking_is_absent_by_default() {
    let body = build_messages_request(&request(), &[]);
    assert!(body.get("thinking").is_none());
}
