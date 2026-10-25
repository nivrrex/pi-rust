//! D5：用 golden 里的 OpenAI 请求体做往返比对。
//!
//! 做法：从 `golden/<场景>/requests.jsonl` 的第一条请求体反推出等价的
//! `ProviderRequest`（system / user 文本 / tools / max_completion_tokens），
//! 用 Rust 的构造器重建，再与 golden **逐字段**比较。
//!
//! 这验证的是"给定 TS 的输入，Rust 产出同一个请求体"——包括裁定 3 要求的
//! "golden 里已出现的 reasoning 字段必须对上"（本批三个场景里这些字段为空集，
//! 见下面的显式断言）。

use pi_ai::openai::{build_chat_completions_request, ToolSpec};
use pi_ai::ProviderRequest;
use serde_json::Value;

mod common;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn first_request_body(scenario: &str) -> Value {
    let path = workspace()
        .join("golden")
        .join(scenario)
        .join("requests.jsonl");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {}: {e}", path.display()));
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .expect("有请求记录");
    let entry: Value = serde_json::from_str(line).expect("请求记录是 JSON");
    let body = entry["body"].as_str().expect("body 是字符串");
    serde_json::from_str(body).expect("请求体是 JSON")
}

/// 从 golden 请求体反推出等价的输入。
fn request_from_body(body: &Value) -> ProviderRequest {
    let messages = body["messages"].as_array().expect("messages 是数组");
    let system = messages
        .iter()
        .find(|m| m["role"] == "system")
        .and_then(|m| m["content"].as_str())
        .map(str::to_string);
    let user = messages
        .iter()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_array())
        .and_then(|blocks| blocks.first())
        .and_then(|block| block["text"].as_str())
        .unwrap_or_default()
        .to_string();

    let tools = body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .map(|tool| ToolSpec {
                    name: tool["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    description: tool["function"]["description"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    parameters: tool["function"]["parameters"].clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    ProviderRequest {
        model: body["model"].as_str().unwrap_or_default().to_string(),
        context: common::ctx_msgs(
            system,
            vec![common::ContextMessage {
                role: common::Role::User,
                content: vec![common::Block::Text { text: user }],
                stop_reason: None,
            }],
        ),
        max_completion_tokens: body["max_completion_tokens"].as_u64().unwrap_or(4096),
        tools,
        thinking: None,
        headers: Vec::new(),
    }
}

fn assert_round_trips(scenario: &str) {
    let golden = first_request_body(scenario);
    let rebuilt = build_chat_completions_request(&request_from_body(&golden));

    assert_eq!(rebuilt["model"], golden["model"], "{scenario}: model");
    assert_eq!(rebuilt["stream"], golden["stream"], "{scenario}: stream");
    assert_eq!(
        rebuilt["stream_options"], golden["stream_options"],
        "{scenario}: stream_options"
    );
    assert_eq!(rebuilt["store"], golden["store"], "{scenario}: store");
    assert_eq!(
        rebuilt["max_completion_tokens"], golden["max_completion_tokens"],
        "{scenario}: max_completion_tokens"
    );
    assert_eq!(
        rebuilt["messages"], golden["messages"],
        "{scenario}: messages"
    );
    assert_eq!(rebuilt["tools"], golden["tools"], "{scenario}: tools");

    let mut rebuilt_keys: Vec<&str> = rebuilt
        .as_object()
        .expect("对象")
        .keys()
        .map(String::as_str)
        .collect();
    let mut golden_keys: Vec<&str> = golden
        .as_object()
        .expect("对象")
        .keys()
        .map(String::as_str)
        .collect();
    rebuilt_keys.sort_unstable();
    golden_keys.sort_unstable();
    assert_eq!(rebuilt_keys, golden_keys, "{scenario}: 字段集合");
}

#[test]
fn s12_openai_request_body_round_trips() {
    assert_round_trips("S12-openai");
}

#[test]
fn s25_openai_request_body_round_trips() {
    assert_round_trips("S25-openai");
}

#[test]
fn s02_request_body_round_trips() {
    assert_round_trips("S02");
}

/// 裁定 3 的要求：golden 里**已出现**的 reasoning 字段必须对上。
///
/// 本批三个场景的请求体里**没有**任何 reasoning 相关字段（`reasoning_effort`、
/// `reasoning`、`thinking` 全为空集），因此 Rust 侧也不应凭空产出它们——
/// 这条断言把"空集"这个事实钉住，避免以后误以为已经覆盖了 reasoning。
#[test]
fn no_reasoning_fields_appear_in_these_goldens() {
    for scenario in ["S12-openai", "S25-openai", "S02"] {
        let golden = first_request_body(scenario);
        for key in ["reasoning_effort", "reasoning", "thinking"] {
            assert!(
                golden.get(key).is_none(),
                "{scenario}: golden 里出现了 {key}，该字段尚未实现，需补"
            );
        }
        let rebuilt = build_chat_completions_request(&request_from_body(&golden));
        for key in ["reasoning_effort", "reasoning", "thinking"] {
            assert!(
                rebuilt.get(key).is_none(),
                "{scenario}: Rust 不应凭空产出 {key}"
            );
        }
    }
}
