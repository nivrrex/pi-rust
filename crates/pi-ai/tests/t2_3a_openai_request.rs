//! T2.3a：`openai-completions` 请求体构造。
//!
//! 判据：字段与顺序对照 `golden/S12-openai/requests.jsonl` 里 TS 实际发出的请求体。

use pi_ai::openai::build_chat_completions_request;
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

fn base_request() -> ProviderRequest {
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
fn request_body_has_the_same_shape_as_the_recorded_one() {
    let body = build_chat_completions_request(&base_request());

    // 只比对字段集合，不比对顺序：serde_json 未启用 preserve_order（启用它需要新增
    // indexmap 依赖，超出允许清单），而 C 层比较是解析后的 JSON，顺序不参与判据。
    let mut keys: Vec<&str> = body
        .as_object()
        .expect("请求体是对象")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    // **没有 `tools` 键**：ref `openai-completions.ts:848-856` 的三分支——有要请求的工具才写；
    // 没有工具但历史里有工具调用/结果时写空数组；两者都不满足时**不写该键**。
    // 本用例既没有工具、上下文里也没有工具历史（golden/S19 的摘要请求同此）。
    assert_eq!(
        keys,
        vec![
            "max_completion_tokens",
            "messages",
            "model",
            "store",
            "stream",
            "stream_options",
        ],
        "顶层字段须与 TS 一致"
    );
    assert_eq!(body["model"], "mock-model");
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["stream_options"]["include_usage"], true,
        "TS 请求 usage"
    );
    assert_eq!(body["store"], false);
    assert_eq!(body["max_completion_tokens"], 4096);
}

#[test]
fn messages_are_system_then_user_with_structured_content() {
    let body = build_chat_completions_request(&base_request());
    let messages = body["messages"].as_array().expect("messages 是数组");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(
        messages[0]["content"],
        "You are an expert coding assistant."
    );
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(
        messages[1]["content"][0]["type"], "text",
        "user 内容是分块数组，不是裸字符串"
    );
    assert_eq!(messages[1]["content"][0]["text"], "think then answer");
}

#[test]
fn system_message_is_omitted_when_absent() {
    let request = ProviderRequest {
        context: ctx_of("think then answer", None),
        ..base_request()
    };
    let body = build_chat_completions_request(&request);
    let messages = body["messages"].as_array().expect("messages 是数组");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
}

/// ref `openai-completions.ts:848-856` 的另外两个分支（本用例是"改写法则"的对照）。
#[test]
fn tools_key_follows_the_ref_three_way_rule() {
    // ① 有要请求的工具 ⇒ 写 `tools`，且是数组
    let mut with_tool = base_request();
    with_tool.tools = vec![pi_ai::openai::ToolSpec {
        name: "read".to_string(),
        description: "读文件".to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
    }];
    let body = build_chat_completions_request(&with_tool);
    assert!(
        body.get("tools")
            .and_then(|value| value.as_array())
            .is_some_and(|tools| !tools.is_empty()),
        "有工具时必须写 tools 且非空"
    );

    // ② 没有工具，但上下文里有工具**结果** ⇒ 写空数组（Anthropic 经代理时要求）
    let mut with_history = base_request();
    with_history
        .context
        .messages
        .push(pi_ai::message::Message::ToolResult {
            tool_call_id: "c1".to_string(),
            tool_name: "read".to_string(),
            content: Vec::new(),
            is_error: false,
            timestamp: 0,
            extra: serde_json::Map::new(),
        });
    let body = build_chat_completions_request(&with_history);
    assert_eq!(
        body.get("tools"),
        Some(&serde_json::json!([])),
        "有工具历史但无工具可请求时必须写空数组"
    );

    // ③ 反证：把工具历史去掉，同一个请求就**不该**有 tools 键（证明②不是别的原因造成的）
    let mut without_history = base_request();
    without_history.tools = Vec::new();
    let body = build_chat_completions_request(&without_history);
    assert!(
        body.get("tools").is_none(),
        "没有工具也没有历史时不得写 tools"
    );
}
