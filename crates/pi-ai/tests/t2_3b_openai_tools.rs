//! T2.3b：`openai-completions` 请求体里的 tools 数组。
//!
//! 判据：`golden/S02` 与 `golden/S25-openai` 的 requests.jsonl。
//! 形状为 `{type:"function", function:{name, description, parameters}}`，
//! `parameters` 是 JSON Schema 的 object 子集。

use pi_ai::openai::{build_chat_completions_request, ToolSpec};
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

fn request_with(tools: Vec<ToolSpec>) -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: ctx_of("hi", Some("sys")),
        max_completion_tokens: 4096,
        tools,
        thinking: None,
        headers: Vec::new(),
    }
}

fn read_tool() -> ToolSpec {
    ToolSpec {
        name: "read".to_string(),
        description: "Read the contents of a file.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {"type": "string", "description": "File path"},
                "offset": {"type": "number", "description": "Start line"}
            }
        }),
    }
}

#[test]
fn tools_are_wrapped_as_function_entries() {
    let body = build_chat_completions_request(&request_with(vec![read_tool()]));
    let tools = body["tools"].as_array().expect("tools 是数组");

    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["function"]["name"], "read");
    assert_eq!(
        tools[0]["function"]["description"],
        "Read the contents of a file."
    );
    assert_eq!(tools[0]["function"]["parameters"]["required"][0], "path");
}

#[test]
fn parameters_are_passed_through_verbatim() {
    let expected = read_tool().parameters.clone();
    let body = build_chat_completions_request(&request_with(vec![read_tool()]));
    let sent = &body["tools"][0]["function"]["parameters"];
    assert_eq!(sent, &expected, "parameters 原样透传，不做增删");
}

#[test]
fn no_tools_and_no_tool_history_omits_the_key() {
    // ref `openai-completions.ts:848-856`：没有工具可请求、历史里也没有工具调用/结果时，
    // **不写 `tools` 键**（我原先写的是"空数组"，那是第二条分支的行为）。
    let body = build_chat_completions_request(&request_with(Vec::new()));
    assert!(
        body.get("tools").is_none(),
        "没有工具也没有工具历史时不得写 tools 键，实际：{:?}",
        body.get("tools")
    );
}

#[test]
fn tool_history_without_requested_tools_yields_an_empty_array() {
    // 同一条 ref 规则的**第二条分支**：历史里有工具调用/结果时必须写空数组。
    let mut request = request_with(Vec::new());
    request
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
    let body = build_chat_completions_request(&request);
    assert_eq!(
        body.get("tools"),
        Some(&serde_json::json!([])),
        "有工具历史但无工具可请求时必须写空数组"
    );
}

#[test]
fn tool_order_follows_the_given_order() {
    let tools: Vec<ToolSpec> = ["read", "bash", "edit", "write"]
        .iter()
        .map(|name| ToolSpec {
            name: (*name).to_string(),
            description: "d".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        })
        .collect();
    let body = build_chat_completions_request(&request_with(tools));
    let names: Vec<&str> = body["tools"]
        .as_array()
        .expect("tools 是数组")
        .iter()
        .map(|t| t["function"]["name"].as_str().expect("name 是字符串"))
        .collect();
    assert_eq!(
        names,
        vec!["read", "bash", "edit", "write"],
        "顺序须与传入一致"
    );
}
