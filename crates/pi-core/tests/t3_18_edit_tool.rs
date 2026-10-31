//! P3.1：`edit` 工具——替换、错误文案、取消、只读失败。

use pi_core::tools::edit::EditTool;
use pi_core::tools::tool::{Tool, ToolContext};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn ctx(dir: &std::path::Path) -> ToolContext {
    ToolContext {
        cwd: dir.to_path_buf(),
    }
}

fn file(dir: &std::path::Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).expect("预置文件");
}

#[tokio::test]
async fn replaces_and_reports_the_block_count() {
    let dir = tempfile::tempdir().expect("临时目录");
    file(dir.path(), "a.txt", "one\ntwo\n");
    let result = EditTool
        .invoke(
            json!({"path": "a.txt", "edits": [{"oldText": "one", "newText": "ONE"}]}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error, "{}", result.text_content());
    assert_eq!(
        result.text_content(),
        "Successfully replaced 1 block(s) in a.txt."
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("可读"),
        "ONE\ntwo\n"
    );
}

#[tokio::test]
async fn multiple_disjoint_edits_are_applied_at_once() {
    let dir = tempfile::tempdir().expect("临时目录");
    file(dir.path(), "a.txt", "one\ntwo\n");
    let result = EditTool
        .invoke(
            json!({"path": "a.txt", "edits": [
                {"oldText": "one", "newText": "1"},
                {"oldText": "two", "newText": "2"}]}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error, "{}", result.text_content());
    assert_eq!(
        result.text_content(),
        "Successfully replaced 2 block(s) in a.txt."
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("可读"),
        "1\n2\n"
    );
}

#[tokio::test]
async fn not_found_reports_the_typescript_text() {
    let dir = tempfile::tempdir().expect("临时目录");
    file(dir.path(), "a.txt", "abc");
    let result = EditTool
        .invoke(
            json!({"path": "a.txt", "edits": [{"oldText": "zzz", "newText": "x"}]}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error);
    assert_eq!(
        result.text_content(),
        "Could not find the exact text in a.txt. The old text must match exactly including all whitespace and newlines."
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("可读"),
        "abc",
        "失败不得改写"
    );
}

#[tokio::test]
async fn duplicate_match_reports_the_occurrence_count() {
    let dir = tempfile::tempdir().expect("临时目录");
    file(dir.path(), "a.txt", "dup\ndup\n");
    let result = EditTool
        .invoke(
            json!({"path": "a.txt", "edits": [{"oldText": "dup", "newText": "x"}]}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error);
    assert!(
        result.text_content().contains("Found 2 occurrences"),
        "{}",
        result.text_content()
    );
}

#[tokio::test]
async fn missing_file_reports_could_not_edit() {
    let dir = tempfile::tempdir().expect("临时目录");
    let result = EditTool
        .invoke(
            json!({"path": "nope.txt", "edits": [{"oldText": "a", "newText": "b"}]}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error);
    let text = result.text_content();
    assert!(
        text.starts_with("Could not edit file: nope.txt. "),
        "{text}"
    );
    assert!(text.contains("ENOENT"), "{text}");
}

#[tokio::test]
async fn empty_edits_array_is_rejected_before_touching_the_file() {
    let dir = tempfile::tempdir().expect("临时目录");
    file(dir.path(), "a.txt", "abc");
    let result = EditTool
        .invoke(
            json!({"path": "a.txt", "edits": []}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error);
    assert_eq!(
        result.text_content(),
        "Edit tool input is invalid. edits must contain at least one replacement."
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("可读"),
        "abc"
    );
}

#[tokio::test]
async fn missing_required_edits_field_is_caught_by_validation() {
    let dir = tempfile::tempdir().expect("临时目录");
    file(dir.path(), "a.txt", "abc");
    let result = EditTool
        .invoke(
            json!({"path": "a.txt"}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error);
    assert!(
        result
            .text_content()
            .contains("must have required properties edits"),
        "{}",
        result.text_content()
    );
}

#[tokio::test]
async fn cancelled_edit_does_not_modify_the_file() {
    let dir = tempfile::tempdir().expect("临时目录");
    file(dir.path(), "a.txt", "abc");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = EditTool
        .invoke(
            json!({"path": "a.txt", "edits": [{"oldText": "abc", "newText": "x"}]}),
            &ctx(dir.path()),
            &cancel,
        )
        .await;
    assert!(result.is_error);
    assert_eq!(result.text_content(), "Operation aborted");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("可读"),
        "abc"
    );
}

#[test]
fn schema_requires_path_and_edits() {
    assert_eq!(EditTool.schema()["required"], json!(["path", "edits"]));
    assert_eq!(
        EditTool.schema()["properties"]["edits"]["items"]["required"],
        json!(["oldText", "newText"])
    );
}
