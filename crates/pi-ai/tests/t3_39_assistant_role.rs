//! P4-5c：assistant 消息的 `role` 键**只能出现一次**，且**必须能读回**。
//!
//! 背景（S27 对拍查出的真 bug）：`Message` 是**内部标签**枚举（`#[serde(tag = "role")]`），
//! `Message::Assistant` 走派生序列化时会写出**重复的 `"role"` 键**（标签一次 + `AssistantMessage`
//! 自带的 `role` 一次）。TS 与我们的比较器都"取最后一个重复键"，所以互操作与差分都掩盖了它；
//! 但**我们自己的 `serde_json::from_str::<Message>` 会报 `duplicate field role`**，
//! 于是 `SessionFile::transcript()` 里的 `from_value(...).ok()` **静默丢掉整条 assistant**——
//! 这正是 S27 请求体里少 assistant 的根因。

use pi_ai::message::Message;

fn assistant_message() -> Message {
    Message::Assistant(Box::new(pi_ai::types::AssistantMessage {
        role: "assistant".to_string(),
        content: vec![pi_ai::Content::Text {
            text: "第一个回答".to_string(),
            text_signature: None,
        }],
        api: "openai-completions".to_string(),
        provider: "mock".to_string(),
        model: "mock-model".to_string(),
        response_model: None,
        usage: pi_ai::Usage::default(),
        stop_reason: pi_ai::StopReason::Stop,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 0,
    }))
}

/// ① 逐字节：`"role":"assistant"` 只出现一次。
///
/// **不能用 `JSON.parse`/`json.loads` 断言**——它们取最后一个重复键，等于没断言。
#[test]
fn assistant_serializes_exactly_one_role_key() {
    let text = serde_json::to_string(&assistant_message()).expect("序列化");
    assert!(
        text.contains(r#""role":"assistant""#),
        "样本必须有 role：{text}"
    );
    assert_eq!(
        text.matches(r#""role":"assistant""#).count(),
        1,
        "`role` 必须只出现一次（重复键会被解析器悄悄吃掉）：{text}"
    );
    assert_eq!(
        text.matches(r#""role""#).count(),
        1,
        "连键名都只能出现一次：{text}"
    );
}

/// ② **必须能读回**——这条是上一次尝试（`skip_serializing`）的判别器：
/// 那个改法让标签消失，读回时认不出变体，消息同样被丢。
#[test]
fn assistant_round_trips_through_serde() {
    let text = serde_json::to_string(&assistant_message()).expect("序列化");
    let parsed: Message = serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!("自己写出的 JSON 必须能读回（否则 transcript() 会静默丢消息）：{error}\n{text}")
    });
    let Message::Assistant(back) = parsed else {
        panic!("读回后仍是 assistant 变体");
    };
    assert_eq!(back.role, "assistant");
    assert_eq!(back.model, "mock-model");
    assert_eq!(back.content.len(), 1, "内容块不能丢");
}

/// ③ 从 `Message` 到**请求体**：assistant 不能在中途被丢掉
/// （S27 的请求体里恰恰少了它——golden 6 条、我们 5 条）。
#[test]
fn assistant_survives_the_openai_conversion() {
    let messages = vec![
        Message::User {
            content: pi_ai::message::UserContent::Text("第一个问题".to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        },
        assistant_message(),
        Message::User {
            content: pi_ai::message::UserContent::Text("继续".to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        },
    ];
    let context = pi_ai::Context::from_messages(None, &messages);
    let out = context.to_openai_messages();
    let roles: Vec<String> = out
        .iter()
        .map(|m| m["role"].as_str().unwrap_or("?").to_string())
        .collect();
    assert_eq!(
        roles,
        vec![
            "user".to_string(),
            "assistant".to_string(),
            "user".to_string()
        ],
        "assistant 必须出现在请求体里（三条顺序不变）：{out:#?}"
    );
    assert!(
        out[1]["content"].as_str() == Some("第一个回答"),
        "assistant 的正文要原样：{:#?}",
        out[1]
    );
}
