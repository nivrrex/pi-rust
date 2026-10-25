//! P4-5b 第1件：`__toolsAddedRaw` / `__sectionsRaw` 是**内部键**，绝不能出现在任何产物里。
//!
//! 背景：为了让 TS 的 `declarationsEqual`（`ref/.../ai/src/utils/transcript.ts:140-141`，
//! 用 `JSON.stringify` 判等）认为工具声明没变，我们把"按 TS 键序的 `toolsAdded`/`sections` 文本"
//! 存在 `extra` 的两个内部键下，由 `Message::System` 的手写 `Serialize` **原样写出**为
//! `toolsAdded`/`sections`。本文件钉住：内部键本身**不落盘、不出现**，读回也不残留。

use pi_ai::message::{Message, UserContent, SECTIONS_RAW_KEY, TOOLS_ADDED_RAW_KEY};

/// 造一条带内部键的 system 消息（形状与生产路径一致）。
fn system_with_raw_keys() -> Message {
    let mut extra = serde_json::Map::new();
    extra.insert("sections".to_string(), serde_json::json!({ "cwd": "/tmp" }));
    extra.insert(
        "toolsAdded".to_string(),
        serde_json::json!([{ "name": "read", "description": "d", "parameters": serde_json::json!({}) }]),
    );
    extra.insert(
        SECTIONS_RAW_KEY.to_string(),
        serde_json::Value::String(r#"{"preamble":"P","cwd":"<cwd>"}"#.to_string()),
    );
    extra.insert(
        TOOLS_ADDED_RAW_KEY.to_string(),
        serde_json::Value::String(
            r#"[{"name":"read","description":"d","parameters":{"type":"object","required":["path"],"properties":{"path":{}}},"constrainedSampling":{"type":"json_schema","strict":"prefer"}}]"#
                .to_string(),
        ),
    );
    Message::System {
        content: UserContent::Text(String::new()),
        timestamp: 0,
        extra,
    }
}

/// **先断言样本含 `toolsAdded`**（对照必须真的触发），再看内部键。
#[test]
fn serialization_has_no_internal_keys_and_keeps_raw_order() {
    let message = system_with_raw_keys();
    let text = serde_json::to_string(&message).expect("序列化");
    // 对照必须真的触发：样本里确实有 toolsAdded 与 sections。
    assert!(text.contains(r#""toolsAdded""#), "样本必须含 toolsAdded");
    assert!(text.contains(r#""sections""#), "样本必须含 sections");
    // 内部键本身**不得出现**。
    assert!(
        !text.contains(TOOLS_ADDED_RAW_KEY),
        "内部键不得落盘：{text}"
    );
    assert!(!text.contains(SECTIONS_RAW_KEY), "内部键不得落盘：{text}");
    // 原样文本被写出来（键序保住、不是被排序的那份）。
    assert!(
        text.contains(r#"{"preamble":"P","cwd":"<cwd>"}"#),
        "sections 原样：{text}"
    );
    assert!(
        text.contains(r#""parameters":{"type":"object","required":["path"]"#),
        "parameters 原样（未被排序）：{text}"
    );
}

/// 读回**自己写出的**文本：`extra` 里不应残留内部键（因为它们没被写出来）。
#[test]
fn reading_back_leaves_no_residue_in_extra() {
    let text = serde_json::to_string(&system_with_raw_keys()).expect("序列化");
    let parsed: Message = serde_json::from_str(&text).expect("反序列化");
    let Message::System { extra, .. } = parsed else {
        panic!("应当是 system 消息");
    };
    assert!(
        !extra.contains_key(TOOLS_ADDED_RAW_KEY),
        "读回后不得残留内部键，实际键：{:?}",
        extra.keys().collect::<Vec<_>>()
    );
    assert!(
        !extra.contains_key(SECTIONS_RAW_KEY),
        "读回后不得残留内部键，实际键：{:?}",
        extra.keys().collect::<Vec<_>>()
    );
    // 未知字段的保真不受影响：这两个键在**读到的文件里**同样应当收进 extra。
    assert!(extra.contains_key("toolsAdded"), "toolsAdded 照旧收着");
    assert!(extra.contains_key("sections"), "sections 照旧收着");
}

/// 读 TS 写的条目（**没有**内部键）：不得凭空造出内部键。
#[test]
fn reading_a_ts_style_entry_does_not_invent_internal_keys() {
    let ts_line = r#"{"role":"system","content":"","timestamp":0,"sections":{"cwd":"/tmp"},"toolsAdded":[{"name":"read","description":"d","parameters":{"properties":{},"type":"object"}}]}"#;
    assert!(ts_line.contains("toolsAdded"), "对照必须真的触发");
    let parsed: Message = serde_json::from_str(ts_line).expect("反序列化");
    let Message::System { extra, .. } = parsed else {
        panic!("应当是 system 消息");
    };
    assert!(!extra.contains_key(TOOLS_ADDED_RAW_KEY));
    assert!(!extra.contains_key(SECTIONS_RAW_KEY));
    // 再把读到的消息写回去：**键序按 extra 里的原样**（BTreeMap）——这是我们在没有原样文本时的行为，
    // 与 TS 的一致性由"只追加不改写"保证（见 decisions：`--continue` 走 `SessionFile::append`）。
    let text = serde_json::to_string(&Message::System {
        content: UserContent::Text(String::new()),
        timestamp: 0,
        extra,
    })
    .expect("序列化");
    assert!(text.contains(r#""toolsAdded""#));
    assert!(!text.contains(TOOLS_ADDED_RAW_KEY));
}

/// P4-5b 第2件对拍发现：assistant 消息曾出现**重复的 `"role"` 键**
/// （`{"role":"assistant","role":"assistant",…}`）——内部标签与 `AssistantMessage` 自带的
/// `role` 字段各写一次。**必须逐字节断言**：`JSON.parse`/`json.loads` 会取最后一个重复键，
/// 用它断言等于没断言（这正是该缺陷长期没被发现的原因）。
#[test]
fn assistant_message_has_exactly_one_role_key() {
    let message = pi_ai::message::Message::Assistant(Box::new(pi_ai::types::AssistantMessage {
        role: "assistant".to_string(),
        content: vec![pi_ai::Content::Text {
            text: "hi".to_string(),
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
    }));
    let text = serde_json::to_string(&message).expect("序列化");
    assert!(
        text.contains(r#""role":"assistant""#),
        "样本必须有 role：{text}"
    );
    assert_eq!(
        text.matches(r#""role":"assistant""#).count(),
        1,
        "`role` 只能出现一次（重复键会被 JSON 解析器悄悄吃掉）：{text}"
    );
    // 反序列化仍然可用（标签提供了 role）。
    let parsed: pi_ai::message::Message = serde_json::from_str(&text).expect("反序列化");
    assert!(matches!(parsed, pi_ai::message::Message::Assistant(_)));
}
