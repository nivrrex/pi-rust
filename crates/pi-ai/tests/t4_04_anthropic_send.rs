//! P4-11 步骤 1.3：anthropic **发送侧**的 thinking 块与 signature。
//!
//! ref `ai/src/api/anthropic-messages.ts`：
//! - `:1330` `hasThinkingSignature = !!sig && sig.trim().length > 0`
//! - `:1331` `if (thinking.trim().length === 0 && !hasThinkingSignature) continue;`（跳过）
//! - `:1335-1347` 无签名时：`allowEmptySignature ? {type:"thinking",…,signature:""} : {type:"text",text:…}`
//!   （`allowEmptySignature` = `model.compat?.allowEmptySignature ?? false`，`:215`；默认 **false**）
//! - `:1348-1352` 有签名时：`{type:"thinking", thinking, signature: thinkingSignature}`
use pi_ai::{Content, Context, Message};

fn assistant(blocks: Vec<Content>) -> Message {
    Message::Assistant(Box::new(pi_ai::AssistantMessage {
        role: "assistant".to_string(),
        content: blocks,
        api: "anthropic-messages".to_string(),
        provider: "mock".to_string(),
        model: "mock-model".to_string(),
        response_model: None,
        usage: Default::default(),
        stop_reason: pi_ai::StopReason::Stop,
        error_message: None,
        raw_stop_reason: None,
        response_id: None,
        timestamp: 0,
        end_turn: None,
    }))
}

fn blocks_of(message: Message) -> Vec<serde_json::Value> {
    let context = Context::from_messages(Some("sys".to_string()), &[message]);
    let messages = context.to_anthropic_messages();
    messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default()
}

fn thinking(text: &str, signature: Option<&str>) -> Content {
    Content::Thinking {
        thinking: text.to_string(),
        thinking_signature: signature.map(str::to_string),
        redacted: None,
    }
}

#[test]
fn thinking_with_signature_is_sent_as_thinking_block() {
    let blocks = blocks_of(assistant(vec![thinking("先读文件再回答", Some("sig-1"))]));
    // 对照必须真的触发：样本里确实有一个带 signature 的 thinking 块。
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    assert_eq!(blocks[0]["type"], "thinking", "{blocks:?}");
    assert_eq!(blocks[0]["thinking"], "先读文件再回答", "{blocks:?}");
    assert_eq!(blocks[0]["signature"], "sig-1", "{blocks:?}");
}

#[test]
fn thinking_without_signature_downgrades_to_text() {
    // 负向（allowEmptySignature 默认 false）：无签名 → text 块（ref `:1343-1346`）
    let blocks = blocks_of(assistant(vec![thinking("先读文件再回答", None)]));
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    assert_eq!(
        blocks[0]["type"], "text",
        "无签名必须降级为 text：{blocks:?}"
    );
    assert_eq!(blocks[0]["text"], "先读文件再回答", "{blocks:?}");
    assert!(blocks[0].get("signature").is_none(), "{blocks:?}");
}

#[test]
fn empty_thinking_without_signature_is_skipped() {
    // 负向：空 thinking 且无签名 → **整块跳过**（ref `:1331`）
    let blocks = blocks_of(assistant(vec![
        thinking("", None),
        Content::Text {
            text: "正文".to_string(),
            text_signature: None,
        },
    ]));
    assert_eq!(blocks.len(), 1, "空格应被跳过：{blocks:?}");
    assert_eq!(blocks[0]["type"], "text", "{blocks:?}");
    assert_eq!(blocks[0]["text"], "正文", "{blocks:?}");
}
