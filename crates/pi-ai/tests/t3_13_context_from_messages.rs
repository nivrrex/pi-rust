//! P3.0：从**会话消息**（`Message`）派生请求上下文，与 golden 的请求体逐项对照。
//!
//! 这是 P3.0 要的"单一源头"路径：会话文件 → `Message` → `Context` → 请求体。
//! 判据是 `golden/S25-anthropic` 的第 3 次请求（含 user / assistant(toolCall) /
//! toolResult / assistant(text) / user(text)）。

use pi_ai::message::Message;
use pi_ai::Context;
use serde_json::Value;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn session_messages(scenario: &str) -> Vec<Message> {
    let path = workspace()
        .join("golden")
        .join(scenario)
        .join("session.jsonl");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读不到 {}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let entry: Value = serde_json::from_str(line).ok()?;
            if entry.get("type").and_then(Value::as_str) != Some("message") {
                return None;
            }
            let message = entry.get("message")?;
            serde_json::from_value(message.clone()).ok()
        })
        .collect()
}

fn golden_request_body(scenario: &str, index: usize) -> Value {
    let path = workspace()
        .join("golden")
        .join(scenario)
        .join("requests.jsonl");
    let text = std::fs::read_to_string(&path).expect("有请求记录");
    let line = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .nth(index)
        .expect("该次请求存在");
    let entry: Value = serde_json::from_str(line).expect("是 JSON");
    serde_json::from_str(entry["body"].as_str().expect("body 是字符串")).expect("请求体是 JSON")
}

#[test]
fn session_messages_derive_the_recorded_request_messages() {
    let messages = session_messages("S25-anthropic");
    assert!(
        messages.len() >= 5,
        "S25-anthropic 的会话里应有 5 条以上消息，实际 {}",
        messages.len()
    );

    // 请求 #3 是**在最后那条 assistant 之前**发出的，因此取值要把它去掉——
    // 这不是"迁就结果"，而是"请求先于响应"这个事实。
    let mut until_request = messages.clone();
    if until_request
        .last()
        .is_some_and(|m| m.role() == "assistant")
    {
        until_request.pop();
    }
    // 系统提示在请求体里是独立字段；这里只比对 messages。
    let context = Context::from_messages(Some("sys".to_string()), &until_request);
    let built = context.to_anthropic_messages();
    let golden = golden_request_body("S25-anthropic", 2);
    let golden_messages = golden["messages"].as_array().expect("golden 有 messages");

    assert_eq!(
        built.len(),
        golden_messages.len(),
        "消息条数应一致：\nbuilt={built:#?}"
    );
    for (index, (mine, theirs)) in built.iter().zip(golden_messages).enumerate() {
        assert_eq!(
            mine["role"],
            theirs["role"],
            "第 {} 条消息的 role 不一致",
            index + 1
        );
        assert_eq!(
            mine["content"],
            theirs["content"],
            "第 {} 条消息的 content 不一致\nmine={}\ntheirs={}",
            index + 1,
            mine["content"],
            theirs["content"]
        );
    }
}

#[test]
fn tool_result_becomes_a_user_block_and_not_a_standalone_role() {
    let messages = session_messages("S25-anthropic");
    assert!(
        messages.iter().any(|m| m.role() == "toolResult"),
        "会话里 toolResult 是独立消息"
    );
    let context = Context::from_messages(None, &messages);
    // P3.0 之后 `Context.messages` 就是 `Message`，判据直接看变体。
    assert!(
        context.messages.iter().all(|m| match m {
            pi_ai::Message::Assistant(assistant) => !assistant.content.is_empty(),
            pi_ai::Message::System { .. }
            | pi_ai::Message::User { .. }
            | pi_ai::Message::ToolResult { .. }
            | pi_ai::Message::Custom { .. }
            | pi_ai::Message::BranchSummary { .. }
            | pi_ai::Message::CompactionSummary { .. } => true,
        }),
        "assistant 消息不应变成空内容"
    );
    // 请求体形状：tool_result 落在 user 消息的块里。
    let built = context.to_anthropic_messages();
    let has_tool_result_block = built.iter().any(|m| {
        m["role"] == "user"
            && m["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"))
    });
    assert!(
        has_tool_result_block,
        "toolResult 应转成 user 消息里的 tool_result 块"
    );
}

#[test]
fn aborted_assistant_is_dropped_by_the_transform() {
    // 会话里放一条 aborted 的 assistant，派生出的请求体不应包含它。
    let messages = serde_json::from_str::<Vec<Message>>(
        r#"[
          {"role":"user","content":"hi","timestamp":1},
          {"role":"assistant","content":[{"type":"text","text":"partial"}],"api":"openai-completions",
           "provider":"mock","model":"mock-model","usage":{"input":0,"output":0,"cacheRead":0,
           "cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
           "stopReason":"aborted","timestamp":2},
          {"role":"user","content":"again","timestamp":3}
        ]"#,
    )
    .expect("消息可解析");

    let context = Context::from_messages(None, &messages);
    let built = context.to_anthropic_messages();
    assert_eq!(built.len(), 2, "aborted 的 assistant 应被丢弃：{built:#?}");
    assert_eq!(built[0]["content"][0]["text"], "hi");
    assert_eq!(built[1]["content"][0]["text"], "again");
}
