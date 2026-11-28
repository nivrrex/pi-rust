//! T2.8：Context 消息到 provider 请求消息的转换（纯函数）。
//!
//! 判据：`golden/S25-anthropic` 第 3 次请求的 messages —— user/assistant tool_use/
//! user tool_result（含 image 块）的真实形状。

use pi_ai::context::Context;
use serde_json::json;

mod common;

fn ctx(blocks: Vec<(common::Role, Vec<common::Block>)>) -> Context {
    common::ctx_msgs(
        None,
        blocks
            .into_iter()
            .map(|(role, content)| common::ContextMessage {
                role,
                content,
                stop_reason: None,
            })
            .collect(),
    )
}

#[test]
fn user_text_becomes_a_text_block() {
    let context = ctx(vec![(
        common::Role::User,
        vec![common::Block::Text {
            text: "看图".to_string(),
        }],
    )]);
    let messages = context.to_anthropic_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"][0]["type"], "text");
    assert_eq!(messages[0]["content"][0]["text"], "看图");
}

#[test]
fn assistant_tool_use_keeps_id_name_and_input() {
    let context = ctx(vec![
        (
            common::Role::User,
            vec![common::Block::Text {
                text: "看图".into(),
            }],
        ),
        (
            common::Role::Assistant,
            vec![common::Block::ToolUse {
                id: "toolu_img".to_string(),
                name: "read".to_string(),
                input: json!({"path": "pic.png"}),
            }],
        ),
    ]);
    let messages = context.to_anthropic_messages();
    let block = &messages[1]["content"][0];
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(block["type"], "tool_use");
    assert_eq!(block["id"], "toolu_img");
    assert_eq!(block["name"], "read");
    assert_eq!(block["input"]["path"], "pic.png");
}

#[test]
fn tool_result_carries_image_blocks_and_error_flag() {
    let context = ctx(vec![(
        common::Role::User,
        vec![common::Block::ToolResult {
            tool_use_id: "toolu_img".to_string(),
            content: vec![
                common::Block::Text {
                    text: "Read image file [image/png]".to_string(),
                },
                common::Block::Image {
                    media_type: "image/png".to_string(),
                    data: "iVBORw0KGgo=".to_string(),
                },
            ],
            is_error: false,
        }],
    )]);
    let messages = context.to_anthropic_messages();
    let block = &messages[0]["content"][0];
    assert_eq!(block["type"], "tool_result");
    assert_eq!(block["tool_use_id"], "toolu_img");
    assert_eq!(block["is_error"], false);
    let content = block["content"].as_array().expect("content 是数组");
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["source"]["type"], "base64");
    assert_eq!(content[1]["source"]["data"], "iVBORw0KGgo=");
    // **位置不同、形状不同**：C2 的 oracle 只覆盖了**顶层 user 图片块**（不带 media_type），
    // 而 `tool_result` 里的图片块**带** media_type——这一点由 golden/S25-anthropic 的
    // "会话 → 请求体" 推导发现（见 `t3_13_context_from_messages`）。
    assert_eq!(
        content[1]["source"]["media_type"], "image/png",
        "tool_result 内的图片块要带 media_type"
    );
}

#[test]
fn last_user_message_gets_cache_control() {
    let context = ctx(vec![
        (
            common::Role::User,
            vec![common::Block::Text {
                text: "看图".into(),
            }],
        ),
        (
            common::Role::Assistant,
            vec![common::Block::Text {
                text: "Hello world".into(),
            }],
        ),
        (
            common::Role::User,
            vec![common::Block::Text {
                text: "再说一句".into(),
            }],
        ),
    ]);
    let messages = context.to_anthropic_messages();
    assert_eq!(
        messages[0]["content"][0].get("cache_control"),
        None,
        "中间轮次不带 cache_control"
    );
    assert_eq!(
        messages[2]["content"][0]["cache_control"]["type"], "ephemeral",
        "最后一轮带 cache_control"
    );
}

#[test]
fn openai_conversion_puts_tool_result_in_a_tool_role_message() {
    let context = ctx(vec![(
        common::Role::User,
        vec![common::Block::ToolResult {
            tool_use_id: "toolu_img".to_string(),
            content: vec![common::Block::Text {
                text: "Read image file [image/png]".to_string(),
            }],
            is_error: false,
        }],
    )]);
    let messages = context.to_openai_messages();
    assert_eq!(messages[0]["role"], "tool");
    assert_eq!(messages[0]["tool_call_id"], "toolu_img");
    assert_eq!(messages[0]["content"], "Read image file [image/png]");
}

/// `Context` 已经接进请求体构造：多轮消息经转换后原样出现在请求里。
#[test]
fn request_body_is_built_from_the_whole_context() {
    use pi_ai::anthropic::build_messages_request;
    use pi_ai::ProviderRequest;

    let context = ctx(vec![
        (
            common::Role::User,
            vec![common::Block::Text {
                text: "看图".into(),
            }],
        ),
        (
            common::Role::Assistant,
            vec![common::Block::ToolUse {
                id: "toolu_img".to_string(),
                name: "read".to_string(),
                input: json!({"path": "pic.png"}),
            }],
        ),
        (
            common::Role::User,
            vec![common::Block::ToolResult {
                tool_use_id: "toolu_img".to_string(),
                content: vec![common::Block::Text {
                    text: "Read image file [image/png]".to_string(),
                }],
                is_error: false,
            }],
        ),
        // golden 的第 3 次请求里最后一轮是纯文本，cache_control 落在它身上。
        (
            common::Role::User,
            vec![common::Block::Text {
                text: "再说一句".into(),
            }],
        ),
    ]);
    let request = ProviderRequest {
        model: "mock-model".to_string(),
        context,
        max_completion_tokens: 4096,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    };
    let body = build_messages_request(&request, &[]);
    let messages = body["messages"].as_array().expect("messages 是数组");

    assert_eq!(messages.len(), 4, "多轮消息原样进入请求体");
    assert_eq!(messages[0]["content"][0]["text"], "看图");
    assert_eq!(messages[1]["content"][0]["type"], "tool_use");
    assert_eq!(messages[2]["content"][0]["type"], "tool_result");
    assert_eq!(
        messages[3]["content"][0]["cache_control"]["type"], "ephemeral",
        "最后一轮 user 的文本块带 cache_control"
    );
    assert_eq!(
        messages[2]["content"][0].get("cache_control"),
        None,
        "含 tool_result 的中间轮次不带 cache_control"
    );
}
