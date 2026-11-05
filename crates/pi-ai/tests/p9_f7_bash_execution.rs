//! P9-F7：`bashExecution` 会话消息（ref `core/messages.ts:25-38`、`:80-99`、`:148-163`）。
//!
//! 覆盖：① 逐字段往返（未知字段保留）；② `!!` 不进上下文、`!` 折成 user 文本；
//! ③ 文本格式与 ref 的 `bashExecutionToText` 一致；④ token 估算与 cut point 语义。

use pi_ai::message::{bash_execution_to_text, to_llm_messages, Message};
use serde_json::json;

fn sample() -> serde_json::Value {
    json!({
        "role": "bashExecution",
        "command": "echo hi",
        "output": "hi\n",
        "exitCode": 0,
        "cancelled": false,
        "truncated": false,
        "timestamp": 1730000000000_i64,
        "unknownField": {"keep": true}
    })
}

#[test]
fn round_trip_keeps_fields_and_unknown_ones() {
    let raw = sample();
    let message: Message = serde_json::from_value(raw.clone()).expect("应能反序列化");
    let back = serde_json::to_value(&message).expect("应能序列化");
    assert_eq!(back["role"], "bashExecution");
    assert_eq!(back["command"], "echo hi");
    assert_eq!(back["output"], "hi\n");
    assert_eq!(back["exitCode"], 0);
    assert_eq!(back["cancelled"], false);
    assert_eq!(back["truncated"], false);
    assert_eq!(back["timestamp"], 1730000000000_i64);
    assert_eq!(
        back["unknownField"],
        json!({"keep": true}),
        "未知字段必须保留"
    );
    assert!(back.get("fullOutputPath").is_none(), "可选字段未给时不写出");
    assert!(back.get("excludeFromContext").is_none());
}

#[test]
fn excluded_message_is_dropped_from_context() {
    let raw = json!({
        "role": "bashExecution", "command": "echo secret", "output": "secret\n",
        "exitCode": 0, "cancelled": false, "truncated": false,
        "timestamp": 1, "excludeFromContext": true
    });
    let message: Message = serde_json::from_value(raw).expect("反序列化");
    assert!(
        to_llm_messages(&[message]).is_empty(),
        "!! 的记录不进上下文"
    );
}

#[test]
fn included_message_becomes_user_text() {
    let raw = sample();
    let message: Message = serde_json::from_value(raw).expect("反序列化");
    let llm = to_llm_messages(&[message]);
    assert_eq!(llm.len(), 1);
    assert_eq!(llm[0].role(), "user");
    let value = serde_json::to_value(&llm[0]).expect("序列化");
    let content = &value["content"];
    let text = content
        .as_str()
        .map(|text| text.to_string())
        .or_else(|| content[0]["text"].as_str().map(|text| text.to_string()))
        .unwrap_or_default();
    assert_eq!(text, "Ran `echo hi`\n```\nhi\n\n```");
}

#[test]
fn text_format_matches_ref_variants() {
    // 无输出
    assert_eq!(
        bash_execution_to_text("true", "", Some(0), false, false, None),
        "Ran `true`\n(no output)"
    );
    // 非零退出码 + 截断
    assert_eq!(
        bash_execution_to_text("false", "boom", Some(2), false, true, Some("/tmp/full.log")),
        "Ran `false`\n```\nboom\n```\n\nCommand exited with code 2\n\n[Output truncated. Full output: /tmp/full.log]"
    );
    // 取消优先于退出码
    assert_eq!(
        bash_execution_to_text("sleep 9", "", None, true, false, None),
        "Ran `sleep 9`\n(no output)\n\n(command cancelled)"
    );
}
