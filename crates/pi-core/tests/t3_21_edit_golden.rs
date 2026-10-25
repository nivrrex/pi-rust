//! P3.2：`edit` 工具对照 golden 的差分测试。
//!
//! 用 `golden/<场景>/session.jsonl` 里 TS 实际发出的工具参数驱动 Rust 的 `EditTool`，
//! 比对三处：`isError`、工具结果文本（逐字）、编辑后各文件的大小。
//! 覆盖 S04（成功）、S04b（找不到）、S04c（不唯一）三个变体。

use pi_core::tools::edit::EditTool;
use pi_core::tools::tool::{Tool, ToolContext};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

/// 场景里预置的文件。
fn scenario_files(id: &str) -> serde_json::Map<String, Value> {
    let path = workspace().join("scenarios").join(format!("{id}.json"));
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("场景可读"))
        .expect("场景是 JSON");
    value
        .get("files")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// golden 里的工具结果：`(is_error, text)` 与工具参数。
fn golden_tool_call(id: &str) -> (bool, String, Value) {
    let path = workspace().join("golden").join(id).join("session.jsonl");
    let text = std::fs::read_to_string(&path).expect("golden 可读");
    let mut arguments = Value::Null;
    let mut result = None;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let entry: Value = serde_json::from_str(line).expect("行是 JSON");
        let Some(message) = entry.get("message") else {
            continue;
        };
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                if let Some(blocks) = message.get("content").and_then(Value::as_array) {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("toolCall") {
                            arguments = block.get("arguments").cloned().unwrap_or(Value::Null);
                        }
                    }
                }
            }
            Some("toolResult") => {
                let content = message
                    .get("content")
                    .and_then(Value::as_array)
                    .and_then(|blocks| blocks.first())
                    .and_then(|block| block.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let is_error = message
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                result = Some((is_error, content));
            }
            Some("system") | Some("user") | None | Some(_) => {}
        }
    }
    let (is_error, content) = result.expect("golden 里有 toolResult");
    (is_error, content, arguments)
}

/// golden 记录的文件大小（文件名 → 字节数）。
fn golden_file_sizes(id: &str) -> Vec<(String, u64)> {
    let path = workspace().join("golden").join(id).join("fs.json");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("fs.json 可读"))
        .expect("fs.json 是 JSON");
    let mut sizes: Vec<(String, u64)> = value
        .as_array()
        .expect("数组")
        .iter()
        .map(|entry| {
            (
                entry["path"].as_str().expect("path").to_string(),
                entry["size"].as_u64().expect("size"),
            )
        })
        .collect();
    sizes.sort();
    sizes
}

fn actual_file_sizes(dir: &std::path::Path) -> Vec<(String, u64)> {
    let mut sizes: Vec<(String, u64)> = std::fs::read_dir(dir)
        .expect("可读目录")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().to_string(),
                entry.metadata().map(|m| m.len()).unwrap_or(0),
            )
        })
        .collect();
    sizes.sort();
    sizes
}

async fn check(id: &str) {
    let dir = tempfile::tempdir().expect("临时目录");
    for (name, content) in scenario_files(id) {
        std::fs::write(
            dir.path().join(&name),
            content.as_str().expect("文件内容是字符串"),
        )
        .expect("预置文件");
    }
    let (expected_is_error, expected_text, arguments) = golden_tool_call(id);

    let result = EditTool
        .invoke(
            arguments,
            &ToolContext {
                cwd: dir.path().to_path_buf(),
            },
            &CancellationToken::new(),
        )
        .await;

    assert_eq!(result.is_error, expected_is_error, "{id}: isError 不一致");
    assert_eq!(
        result.text_content(),
        expected_text,
        "{id}: 工具结果文本必须与 golden 逐字一致"
    );
    assert_eq!(
        actual_file_sizes(dir.path()),
        golden_file_sizes(id),
        "{id}: 编辑后的文件大小必须与 golden 一致"
    );
}

#[tokio::test]
async fn s04_successful_edit_matches_golden() {
    check("S04").await;
}

#[tokio::test]
async fn s04b_not_found_matches_golden_and_leaves_the_file_alone() {
    check("S04b").await;
}

#[tokio::test]
async fn s04c_duplicate_matches_golden_and_leaves_the_file_alone() {
    check("S04c").await;
}
