//! P3-6 / D6：`Message` 枚举——四个成员的联合，含**独立的 `ToolResult` 变体**。
//!
//! 判据：`spec/a-types.md` 第 8–15 节，以及 `golden/S25-anthropic/session.jsonl`
//! 里 TS 实际写下的四种 role。

use pi_ai::message::{Content, Message, ToolResultContent};
use serde_json::Value;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn golden_sessions(scenario: &str) -> Vec<Value> {
    let path = workspace()
        .join("golden")
        .join(scenario)
        .join("session.jsonl");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读不到 {}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("会话行是 JSON"))
        .collect()
}

#[test]
fn four_roles_parse_from_golden_messages() {
    // 仅取形如 `Message` 的条目：会话文件里还混有 provider 级负载（无 `role`）。
    let mut seen: Vec<&'static str> = Vec::new();
    for entry in golden_sessions("S25-anthropic") {
        let Some(message) = entry.get("message") else {
            continue;
        };
        let Some(role) = message.get("role").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(role, "system" | "user" | "assistant" | "toolResult") {
            continue;
        }
        let parsed: Message = serde_json::from_value(message.clone()).unwrap_or_else(|e| {
            panic!("解析失败（role={role}）：{e}\n{message}");
        });
        assert_eq!(parsed.role(), role, "role 须与原文一致");
        if !seen.contains(&parsed.role()) {
            seen.push(parsed.role());
        }
    }
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec!["assistant", "system", "toolResult", "user"],
        "四种 role 都要能解析"
    );
}

#[test]
fn tool_result_is_a_standalone_variant_not_a_content_block() {
    let entries = golden_sessions("S25-anthropic");
    let tool_result = entries
        .iter()
        .filter_map(|entry| entry.get("message"))
        .find(|message| message.get("role").and_then(Value::as_str) == Some("toolResult"))
        .expect("golden 里有 toolResult 消息");

    let parsed: Message = serde_json::from_value(tool_result.clone()).expect("可解析");
    match parsed {
        Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            is_error,
            ..
        } => {
            assert_eq!(tool_call_id, "toolu_img");
            assert!(!tool_name.is_empty(), "toolName 不能丢");
            assert!(!is_error, "golden 里这条不是错误结果");
            assert!(!content.is_empty(), "内容块要保留");
        }
        other => panic!("toolResult 必须是独立变体，实际 {:?}", other.role()),
    }
}

#[test]
fn tool_result_content_supports_text_and_image() {
    let entries = golden_sessions("S25-anthropic");
    let tool_result = entries
        .iter()
        .filter_map(|entry| entry.get("message"))
        .find(|message| message.get("role").and_then(Value::as_str) == Some("toolResult"))
        .expect("有 toolResult");
    let parsed: Message = serde_json::from_value(tool_result.clone()).expect("可解析");
    let Message::ToolResult { content, .. } = parsed else {
        panic!("应为 ToolResult");
    };
    let mut kinds: Vec<&str> = content
        .iter()
        .map(|block| match block {
            ToolResultContent::Text { .. } => "text",
            ToolResultContent::Image { .. } => "image",
        })
        .collect();
    kinds.sort_unstable();
    assert_eq!(kinds, vec!["image", "text"], "工具结果可同时含文本与图片");
}

#[test]
fn unknown_fields_are_preserved_for_interop() {
    let raw = serde_json::json!({
        "role": "user",
        "content": "hi",
        "timestamp": 1,
        "futureField": {"nested": true}
    });
    let parsed: Message = serde_json::from_value(raw.clone()).expect("可解析");
    let written = serde_json::to_value(&parsed).expect("可写回");
    assert_eq!(
        written["futureField"], raw["futureField"],
        "未知字段必须保留"
    );
}

#[test]
fn message_content_accepts_plain_string_and_blocks() {
    let as_string: Message = serde_json::from_value(
        serde_json::json!({"role": "user", "content": "hi", "timestamp": 1}),
    )
    .expect("字符串形态可解析");
    let as_blocks: Message = serde_json::from_value(serde_json::json!({
        "role": "user",
        "content": [{"type": "text", "text": "hi"}],
        "timestamp": 1
    }))
    .expect("块数组形态可解析");
    assert_eq!(as_string.role(), "user");
    assert_eq!(as_blocks.role(), "user");
    let Content::Text { text, .. } = &as_blocks.blocks()[0] else {
        panic!("应为文本块");
    };
    assert_eq!(text, "hi");
}
