//! P4-8 步骤 1：发送侧按 `thinkingSignature` 回填 reasoning 字段。
//!
//! ref `ai/src/api/openai-completions.ts:1331-1342`：
//! - `:1334` `let signature = nonEmptyThinkingBlocks[0].thinkingSignature;`
//! - `:1335-1337` `model.provider === "opencode-go" && signature === "reasoning"` → `signature = "reasoning_content"`
//! - `:1338-1340` `if (signature && isOpenAICompletionsReasoningField(signature))`
//!   → `assistantMsg[signature] = 非空 thinking 块用 "\n" 连接`
//! - 合法字段名（`:268-274`）：`reasoning` / `reasoning_content` / `reasoning_text`
//! - 整段被 `if (!preservedReasoningDetails)` 包着（`:1332`）
use pi_ai::{Content, Context, Message, ProviderRequest};

fn assistant_with_thinking(signature: Option<&str>) -> Message {
    Message::Assistant(Box::new(pi_ai::AssistantMessage {
        role: "assistant".to_string(),
        content: vec![
            Content::Thinking {
                thinking: "先读文件再回答".to_string(),
                thinking_signature: signature.map(str::to_string),
                redacted: None,
            },
            Content::Text {
                text: "答案".to_string(),
                text_signature: None,
            },
        ],
        api: "openai-completions".to_string(),
        provider: "mock".to_string(),
        model: "mock-model".to_string(),
        response_model: None,
        usage: Default::default(),
        response_id: None,
        timestamp: 0,
        end_turn: None,
        stop_reason: pi_ai::StopReason::Stop,
        error_message: None,
        raw_stop_reason: None,
    }))
}

fn request(messages: &[Message]) -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: Context::from_messages(Some("sys".to_string()), messages),
        max_completion_tokens: 16,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    }
}

fn body_with_provider(provider: &str, signature: Option<&str>) -> serde_json::Value {
    let request = request(&[assistant_with_thinking(signature)]);
    request
        .context
        .to_openai_messages_with_provider(provider)
        .into_iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .expect("必须有一条 assistant 消息")
}

#[test]
fn reasoning_content_is_sent_back_when_the_signature_says_so() {
    let body = body_with_provider("mock", Some("reasoning_content"));
    // 对照必须真的触发：样本里确实有一个**带 signature** 的 thinking 块。
    assert_eq!(
        body.get("reasoning_content").and_then(|v| v.as_str()),
        Some("先读文件再回答"),
        "signature=reasoning_content 时必须回填同名字段：{body}"
    );
}

#[test]
fn opencode_go_maps_reasoning_to_reasoning_content() {
    // ref `:1335-1337` 的特例。
    let body = body_with_provider("opencode-go", Some("reasoning"));
    assert_eq!(
        body.get("reasoning_content").and_then(|v| v.as_str()),
        Some("先读文件再回答"),
        "opencode-go + signature=reasoning 时应写 reasoning_content：{body}"
    );
    assert!(
        body.get("reasoning").is_none(),
        "特例生效时不应再写 reasoning 字段：{body}"
    );
}

#[test]
fn non_opencode_go_does_not_apply_the_special_case() {
    // 负向用例（发送侧）：非 opencode-go 时 signature=reasoning **原样**用作字段名。
    let body = body_with_provider("mock", Some("reasoning"));
    assert_eq!(
        body.get("reasoning").and_then(|v| v.as_str()),
        Some("先读文件再回答"),
        "非 opencode-go 必须原样使用 signature 作字段名：{body}"
    );
    assert!(
        body.get("reasoning_content").is_none(),
        "非 opencode-go 不得套用特例：{body}"
    );
}

#[test]
fn missing_or_unknown_signature_emits_no_reasoning_field() {
    // ref `:1338` 的 `if (signature && …)`：signature 为空/缺失 → **不写任何字段**。
    for signature in [None, Some(""), Some("不是合法字段")] {
        let body = body_with_provider("mock", signature);
        for field in ["reasoning", "reasoning_content", "reasoning_text"] {
            assert!(
                body.get(field).is_none(),
                "signature={signature:?} 时不应写 {field}：{body}"
            );
        }
    }
}
