//! P4-5a 丙第二批：保序渲染器的测试。
//!
//! oracle = 各工具 schema 的**源码键序**（就是 TS `toToolDeclaration` 的键序，
//! 见 `ref/.../ai/src/utils/transcript.ts:123-130`）。

use pi_core::tools::schema_order::{render_ordered, OrderRule};

/// `read` 的键序表（照 `tools/read.rs:21-31` 的源码顺序抄键名）。
const READ_RULES: &[OrderRule] = &[
    (&[], &["type", "required", "properties"]),
    (&["properties"], &["path", "offset", "limit"]),
];

/// 逐字节等于 TS 的键序（`read`）。
#[test]
fn read_schema_renders_in_ts_key_order() {
    let schema = pi_core::tools::read::schema();
    let text = render_ordered(&schema, READ_RULES);
    assert!(
        text.starts_with(
            r#"{"type":"object","required":["path"],"properties":{"path":{"type":"string""#
        ),
        "顶层与首个属性的键序必须与 TS 一致，实际：{}",
        &text[..text.len().min(160)]
    );
    // 负向对照：排序后的形态（BTreeMap 会产出的那种）**不应**出现。
    assert!(
        !text.starts_with(r#"{"properties""#),
        "不应出现被排序后的形态：{}",
        &text[..text.len().min(80)]
    );
    // 语义必须与 `Value` 一致（渲染只是换顺序）。
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("渲出来的是合法 JSON");
    assert_eq!(parsed, schema, "语义必须与 schema() 完全相同");
}

/// `bash` 的键序表（照 `tools/bash.rs:33-42`）。
const BASH_RULES: &[OrderRule] = &[
    (&[], &["type", "required", "properties"]),
    (&["properties"], &["command", "timeout"]),
];

#[test]
fn bash_schema_renders_in_ts_key_order() {
    let schema = pi_core::tools::bash::schema();
    let text = render_ordered(&schema, BASH_RULES);
    assert!(
        text.starts_with(
            r#"{"type":"object","required":["command"],"properties":{"command":{"type":"string""#
        ),
        "实际：{}",
        &text[..text.len().min(160)]
    );
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("合法 JSON");
    assert_eq!(parsed, schema);
}

/// `edit` 的键序表：顶层 `type, required, properties`；`properties` 是 `path, edits`；
/// `edits` 属性对象是 `type, description, items`；`items` 是 `type, required, properties`；
/// `items.properties` 是 `oldText, newText`（照 `tools/edit.rs:21-42`）。
const EDIT_RULES: &[OrderRule] = &[
    (&[], &["type", "required", "properties"]),
    (&["properties"], &["path", "edits"]),
    (&["properties", "edits"], &["type", "description", "items"]),
    (
        &["properties", "edits", "items"],
        &["type", "required", "properties"],
    ),
    (
        &["properties", "edits", "items", "properties"],
        &["oldText", "newText"],
    ),
];

#[test]
fn edit_schema_renders_in_ts_key_order() {
    let schema = pi_core::tools::edit::schema();
    let text = render_ordered(&schema, EDIT_RULES);
    assert!(
        text.starts_with(
            r#"{"type":"object","required":["path","edits"],"properties":{"path":{"type":"string""#
        ),
        "实际：{}",
        &text[..text.len().min(160)]
    );
    assert!(
        text.contains(
            r#""items":{"type":"object","required":["oldText","newText"],"properties":{"oldText""#
        ),
        "items 内部的键序也要对，实际：{}",
        &text[text.len().saturating_sub(300)..]
    );
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("合法 JSON");
    assert_eq!(parsed, schema);
}

/// `write` 的键序表（照 `tools/write.rs:18-27`）。
const WRITE_RULES: &[OrderRule] = &[
    (&[], &["type", "required", "properties"]),
    (&["properties"], &["path", "content"]),
];

#[test]
fn write_schema_renders_in_ts_key_order() {
    let schema = pi_core::tools::write::schema();
    let text = render_ordered(&schema, WRITE_RULES);
    assert!(
        text.starts_with(r#"{"type":"object","required":["path","content"],"properties":{"path":{"type":"string""#),
        "实际：{}",
        &text[..text.len().min(160)]
    );
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("合法 JSON");
    assert_eq!(parsed, schema);
}

/// 规则没覆盖到的键**不能丢**（接在规则键之后，保持原顺序）。
#[test]
fn keys_outside_the_rules_are_appended_not_dropped() {
    let value: serde_json::Value =
        serde_json::from_str(r#"{"zzz":1,"type":"object","aaa":2,"required":[]}"#).expect("JSON");
    let text = render_ordered(&value, &[(&[], &["type", "required"])]);
    assert!(
        text.starts_with(r#"{"type":"object","required":[]"#),
        "实际：{text}"
    );
    assert!(text.contains(r#""aaa":2"#), "未列出的键不能丢：{text}");
    assert!(text.contains(r#""zzz":1"#), "未列出的键不能丢：{text}");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("合法 JSON");
    assert_eq!(parsed, value, "顺序变了、语义不能变");
}
