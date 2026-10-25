//! P4-12 B1：anthropic 请求体必须真的带上 `tools`（此前 `anthropic.rs:306` 硬编码 `&[]`）。
//!
//! ref `ai/src/api/anthropic-messages.ts`：
//! `:1122` / `:1141` `params.tools = convertTools(...)`（有工具时）
//! `:209` `supportsEagerToolInputStreaming: model.compat?.supportsEagerToolInputStreaming ?? true`
//! `types.ts:780-786`：该 compat **为 false 时省略** `tools[].eager_input_streaming`
//! golden/S12-anthropic 的 `tools[0]` 键为 `name` / `description` / `eager_input_streaming` / `input_schema`
use pi_ai::{Content, Context, Message, ProviderRequest};

fn request_with_tools() -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: Context::from_messages(
            Some("sys".to_string()),
            &[Message::User {
                content: pi_ai::UserContent::Text("hi".to_string()),
                timestamp: 0,
                extra: serde_json::Map::new(),
            }],
        ),
        max_completion_tokens: 16,
        tools: vec![pi_ai::openai::ToolSpec {
            name: "read".to_string(),
            description: "读文件".to_string(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
        }],
        thinking: None,
        headers: Vec::new(),
    }
}

#[test]
fn anthropic_request_carries_tools() {
    let request = request_with_tools();
    let _ = Content::Text {
        text: String::new(),
        text_signature: None,
    };
    // 对照必须真的触发：request 里确实有一个工具。
    assert_eq!(request.tools.len(), 1);
    let body = pi_ai::anthropic::build_messages_request_with_tools(&request, request.thinking);
    let tools = body.get("tools").and_then(|t| t.as_array());
    assert!(tools.is_some(), "请求体必须带 tools：{body}");
    let tools = tools.expect("已断言存在");
    assert_eq!(tools.len(), 1, "{tools:?}");
    let tool = &tools[0];
    // ref `:1141` 的序列化：`name` / `description` / `eager_input_streaming` / `input_schema`
    assert_eq!(tool["name"], "read", "{tool}");
    assert_eq!(tool["description"], "读文件", "{tool}");
    // ref `:209` 的默认 true
    assert_eq!(tool["eager_input_streaming"], true, "{tool}");
    assert_eq!(
        tool["input_schema"],
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
        "{tool}"
    );
}

#[test]
fn anthropic_request_without_tools_omits_the_field() {
    // 负向：没有工具时不发 `tools`（ref `:1139-1140` 的 `if (tools.length > 0)`）
    let mut request = request_with_tools();
    request.tools.clear();
    let body = pi_ai::anthropic::build_messages_request_with_tools(&request, request.thinking);
    assert!(
        body.get("tools").is_none(),
        "无工具时不应出现 tools：{body}"
    );
}
