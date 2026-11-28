//! P4-13 步骤 4.1：anthropic 请求体里 `tools[].cache_control` 的位置。
//!
//! ref `api/anthropic-messages.ts`：
//! - `:213` `supportsCacheControlOnTools: model.compat?.supportsCacheControlOnTools ?? true`
//! - `:1114` `const toolCacheControl = compat.supportsCacheControlOnTools ? cacheControl : undefined;`
//! - `:1489` `...(cacheControl && index === tools.length - 1 ? { cache_control: cacheControl } : {})`
//!   → **只有最后一个工具**带 `cache_control`
//! - `:1486` `...(supportsEagerToolInputStreaming ? { eager_input_streaming: true } : {})`
//!
//! golden/S12-anthropic 的实测：`tools[0..2]` **没有** `cache_control`，最后一个（write）**有**。
use pi_ai::anthropic::{build_messages_request, ToolSpecAnthropic};
use pi_ai::{Context, Message, ProviderRequest};

fn tool(name: &str) -> ToolSpecAnthropic {
    ToolSpecAnthropic {
        name: name.to_string(),
        description: format!("{name} 工具"),
        input_schema: serde_json::json!({"type":"object","properties":{}}),
        eager_input_streaming: true,
    }
}

fn request() -> ProviderRequest {
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
        max_completion_tokens: 4096,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    }
}

#[test]
fn only_the_last_tool_carries_cache_control() {
    let request = request();
    let tools = vec![tool("read"), tool("bash"), tool("write")];
    let body = build_messages_request(&request, tools.as_slice());
    let sent = body["tools"].as_array().expect("tools 是数组");
    assert_eq!(sent.len(), 3, "样本自检：确实发了三个工具");
    assert!(
        sent[0].get("cache_control").is_none(),
        "第一个工具不应带 cache_control：{}",
        sent[0]
    );
    assert!(
        sent[1].get("cache_control").is_none(),
        "第二个工具不应带 cache_control：{}",
        sent[1]
    );
    assert_eq!(
        sent[2]["cache_control"],
        serde_json::json!({"type":"ephemeral"}),
        "**最后一个**工具才带 cache_control：{}",
        sent[2]
    );
}

#[test]
fn single_tool_is_also_the_last_one() {
    let request = request();
    let body = build_messages_request(&request, &[tool("read")]);
    let sent = body["tools"].as_array().expect("tools 是数组");
    assert_eq!(
        sent[0]["cache_control"],
        serde_json::json!({"type":"ephemeral"})
    );
}

#[test]
fn no_tool_gets_cache_control_when_the_context_disables_it() {
    // 负向对照：`cacheControl` 不存在时整段不发（ref `:1489` 的 `cacheControl &&`）。
    let mut request = request();
    request.context.cache_control_system = false;
    let body = build_messages_request(&request, &[tool("read"), tool("write")]);
    let sent = body["tools"].as_array().expect("tools 是数组");
    for entry in sent {
        assert!(
            entry.get("cache_control").is_none(),
            "关掉缓存时不应带 cache_control：{entry}"
        );
    }
}

#[test]
fn eager_input_streaming_stays_on_every_tool() {
    // ref `:1486` 与 `:1489` 是两回事：eager 是**每个**工具都发。
    let request = request();
    let body = build_messages_request(&request, &[tool("read"), tool("write")]);
    let sent = body["tools"].as_array().expect("tools 是数组");
    for entry in sent {
        assert_eq!(entry["eager_input_streaming"], true, "{entry}");
    }
}
