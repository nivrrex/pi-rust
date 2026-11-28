//! P3.1：工具参数的 schema 校验——行为与错误文案以 golden 为准。
//!
//! 判据：`golden/S26`（缺必填的原文）、`golden/S26b`/`S26c`（类型错与多余字段**不报错**）。

use pi_core::tools::schema::validate_required;
use serde_json::json;

fn read_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "required": ["path"],
        "properties": {
            "path": {"type": "string"},
            "offset": {"type": "number"}
        }
    })
}

#[test]
fn missing_required_produces_the_exact_typescript_text() {
    let error = validate_required("read", &read_schema(), &json!({})).expect_err("应报错");
    let golden = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("工作区根")
            .join("golden/S26/requests.jsonl"),
    )
    .expect("golden/S26 存在");
    // 从 golden 的第二次请求里取出 toolResult 的正文，逐字比对。
    let expected = golden
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|entry| serde_json::from_str::<serde_json::Value>(entry["body"].as_str()?).ok())
        .and_then(|body| {
            body["messages"]
                .as_array()?
                .iter()
                .find(|m| m["role"] == "tool")?
                .get("content")?
                .as_str()
                .map(str::to_string)
        })
        .expect("golden 里有 tool 消息");

    assert_eq!(error.message, expected, "错误文案必须与 TS 逐字一致");
    assert_eq!(error.missing, vec!["path".to_string()]);
}

#[test]
fn wrong_type_is_accepted_because_typescript_accepts_it() {
    // golden/S26b：`{"path": 123}` 没有触发校验错误，而是把 123 当路径去 access。
    assert!(
        validate_required("read", &read_schema(), &json!({"path": 123})).is_ok(),
        "TS 不校验类型，我们也不能校验——否则会产出 TS 不会产出的错误"
    );
}

#[test]
fn extra_fields_are_accepted_because_typescript_ignores_them() {
    // golden/S26c：多余字段被忽略。
    assert!(
        validate_required(
            "read",
            &read_schema(),
            &json!({"path": "a.txt", "whatever": true})
        )
        .is_ok(),
        "TS 忽略多余字段"
    );
}

#[test]
fn schema_without_required_accepts_anything() {
    assert!(validate_required("x", &json!({"type": "object"}), &json!({})).is_ok());
}

#[test]
fn multiple_missing_fields_are_each_listed() {
    let schema = json!({"type": "object", "required": ["path", "content"]});
    let error = validate_required("write", &schema, &json!({})).expect_err("应报错");
    assert!(error
        .message
        .contains("  - path: must have required properties path\n"));
    assert!(error
        .message
        .contains("  - content: must have required properties content\n"));
    assert_eq!(error.missing.len(), 2);
}

#[test]
fn non_object_arguments_count_as_missing_everything() {
    let error =
        validate_required("read", &read_schema(), &json!("not an object")).expect_err("应报错");
    assert_eq!(error.missing, vec!["path".to_string()]);
}

/// 四个工具的校验文案都对着 golden 逐字比对（S26=read、S26d=write、S26e=edit、S26f=bash）。
#[test]
fn every_builtin_tool_validation_text_matches_golden() {
    let cases = [
        (
            "S26",
            "read",
            json!({"type": "object", "required": ["path"]}),
            json!({}),
        ),
        (
            "S26d",
            "write",
            json!({"type": "object", "required": ["path", "content"]}),
            json!({"path": "a.txt"}),
        ),
        (
            "S26e",
            "edit",
            json!({"type": "object", "required": ["path", "edits"]}),
            json!({"path": "a.txt"}),
        ),
        (
            "S26f",
            "bash",
            json!({"type": "object", "required": ["command"]}),
            json!({}),
        ),
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("工作区根")
        .to_path_buf();
    for (scenario, tool, schema, arguments) in cases {
        let golden =
            std::fs::read_to_string(root.join(format!("golden/{scenario}/requests.jsonl")))
                .unwrap_or_else(|e| panic!("读不到 golden/{scenario}: {e}"));
        let expected = golden
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .and_then(|entry| {
                serde_json::from_str::<serde_json::Value>(entry["body"].as_str()?).ok()
            })
            .and_then(|body| {
                body["messages"]
                    .as_array()?
                    .iter()
                    .find(|m| m["role"] == "tool")?
                    .get("content")?
                    .as_str()
                    .map(str::to_string)
            })
            .unwrap_or_else(|| panic!("golden/{scenario} 里有 tool 消息"));

        let error = validate_required(tool, &schema, &arguments).expect_err("应报错");
        assert_eq!(
            error.message, expected,
            "{scenario}（{tool}）的文案必须逐字一致"
        );
    }
}
