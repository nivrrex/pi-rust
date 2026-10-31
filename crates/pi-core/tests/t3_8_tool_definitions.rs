//! P3-10：四个内置工具的定义与 TS 发出的 tools 数组逐字段一致。
//!
//! 判据：`golden/<场景>/requests.jsonl` 里 TS 实际发出的 `tools`。

use pi_core::tools::definitions::builtin_tools;
use serde_json::Value;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn golden_tools(scenario: &str) -> Value {
    let path = workspace()
        .join("golden")
        .join(scenario)
        .join("requests.jsonl");
    let line = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读不到 {}: {e}", path.display()))
        .lines()
        .find(|l| !l.trim().is_empty())
        .expect("有请求记录")
        .to_string();
    let entry: Value = serde_json::from_str(&line).expect("记录是 JSON");
    let body: Value = serde_json::from_str(entry["body"].as_str().expect("body 是字符串"))
        .expect("请求体是 JSON");
    body["tools"].clone()
}

fn rust_tools() -> Value {
    Value::Array(
        builtin_tools()
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect(),
    )
}

#[test]
fn builtin_tools_match_the_golden_tools_array() {
    for scenario in ["S02", "S12-openai", "S25-openai"] {
        assert_eq!(
            rust_tools(),
            golden_tools(scenario),
            "{scenario}: 工具定义必须与 TS 发出的完全一致"
        );
    }
}

#[test]
fn tool_order_is_read_bash_edit_write() {
    let names: Vec<String> = builtin_tools().into_iter().map(|tool| tool.name).collect();
    assert_eq!(names, vec!["read", "bash", "edit", "write"]);
}

#[test]
fn every_tool_declares_a_required_parameter() {
    for tool in builtin_tools() {
        let required = tool.parameters["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{} 缺少 required", tool.name));
        assert!(!required.is_empty(), "{} 的 required 不应为空", tool.name);
        let properties = tool.parameters["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{} 缺少 properties", tool.name));
        for name in required {
            let name = name.as_str().expect("required 是字符串");
            assert!(
                properties.contains_key(name),
                "{} 的 required 里有未声明的字段 {name}",
                tool.name
            );
        }
    }
}
