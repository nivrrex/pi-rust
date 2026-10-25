//! P4-11 步骤 1.2：anthropic 解码侧的 thinking / signature / redacted_thinking。
//!
//! ref `ai/src/api/anthropic-messages.ts`：
//! - `content_block_start` 的 `thinking` 分支 `:641-649`（块字段 `:643-646`，其中
//!   **`thinkingSignature: event.content_block.signature ?? ""` 在 `:645`**；事件 `thinking_start` `:649`）
//! - `content_block_start` 的 `redacted_thinking` 分支 `:650-659`（`thinking: "[Reasoning redacted]"` `:653`、
//!   `thinkingSignature: event.content_block.data` `:654`、`redacted: true` `:655`）
//! - `content_block_delta` 的 `thinking_delta` `:687-695`（累加 + 事件 `:693-695`）
//! - `content_block_delta` 的 `signature_delta` `:712-717`（`|| ""` 再 `+=`；**不发事件**）
//! - `content_block_stop` 的 thinking 分支 → `thinking_end` `:732-735`
use pi_ai::decoder::{Rates, StreamDecoder};

fn sse(lines: &[&str]) -> Vec<u8> {
    let mut text = String::new();
    for line in lines {
        text.push_str("data: ");
        text.push_str(line);
        text.push('\n');
        text.push('\n');
    }
    text.into_bytes()
}

fn decode(lines: &[&str]) -> (serde_json::Value, String) {
    let mut d = StreamDecoder::new("anthropic-messages", "mock", "mock-model", Rates::zero())
        .expect("decoder 可构造");
    let events = {
        let mut all = Vec::new();
        all.extend(d.push(&sse(lines)).expect("push"));
        all.extend(d.finish().expect("finish"));
        all
    };
    (
        serde_json::to_value(d.final_message()).expect("消息可序列化"),
        format!("{events:?}"),
    )
}

#[test]
fn thinking_block_keeps_signature_and_emits_thinking_events() {
    let start = r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":"sig-1"}}"#;
    let delta = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"先读文件"}}"#;
    let sig = r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"-tail"}}"#;
    let stop = r#"{"type":"content_block_stop","index":0}"#;
    // 对照必须真的触发：样本里确实是 thinking 块 + signature_delta。
    assert!(start.contains(r#""type":"thinking""#) && sig.contains("signature_delta"));

    let (message, events) = decode(&[start, delta, sig, stop]);
    assert!(
        events.contains("ThinkingStart") && events.contains("ThinkingDelta"),
        "应发 thinking_start/thinking_delta（当前落到 text）：{events}"
    );
    assert!(
        !events.contains("TextStart"),
        "不得把它当 text 块：{events}"
    );
    let content = &message["content"][0];
    assert_eq!(content["type"], "thinking", "{message}");
    assert_eq!(content["thinking"], "先读文件", "{message}");
    // `:645` 的缺省值 + `:716-717` 的累加
    assert_eq!(content["thinkingSignature"], "sig-1-tail", "{message}");
}

#[test]
fn signature_delta_without_prior_signature_starts_from_empty() {
    // ref `:716` 的 `block.thinkingSignature || ""`
    let start = r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#;
    let sig = r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"only"}}"#;
    let (message, _) = decode(&[start, sig]);
    assert_eq!(
        message["content"][0]["thinkingSignature"], "only",
        "{message}"
    );
}

#[test]
fn redacted_thinking_becomes_a_thinking_block_with_the_data_as_signature() {
    // ref `:650-659`
    let start = r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"opaque-abc"}}"#;
    assert!(start.contains("redacted_thinking"));
    let (message, events) = decode(&[start]);
    let content = &message["content"][0];
    assert_eq!(content["type"], "thinking", "{message}");
    assert_eq!(content["thinking"], "[Reasoning redacted]", "{message}");
    assert_eq!(content["thinkingSignature"], "opaque-abc", "{message}");
    assert_eq!(content["redacted"], true, "{message}");
    assert!(events.contains("ThinkingStart"), "{events}");
}

#[test]
fn text_block_is_not_turned_into_thinking_and_ignores_signature_delta() {
    // 负向：text 块 + signature_delta（ref `:715` 的 `block.type === "thinking"` 守卫）
    let start =
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
    let sig = r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"nope"}}"#;
    let text =
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"正文"}}"#;
    let (message, events) = decode(&[start, sig, text]);
    let content = &message["content"][0];
    assert_eq!(content["type"], "text", "{message}");
    assert_eq!(content["text"], "正文", "{message}");
    assert!(content.get("thinkingSignature").is_none(), "{message}");
    assert!(
        events.contains("TextStart") && !events.contains("ThinkingStart"),
        "{events}"
    );
}
