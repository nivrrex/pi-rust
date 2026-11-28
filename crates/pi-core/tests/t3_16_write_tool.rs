//! P3.1：`write` 工具——建父目录、覆盖、成功文案、失败路径。

use pi_core::tools::tool::{Tool, ToolContext};
use pi_core::tools::write::WriteTool;
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn ctx(dir: &std::path::Path) -> ToolContext {
    ToolContext {
        cwd: dir.to_path_buf(),
    }
}

#[tokio::test]
async fn writes_a_new_file_and_creates_parent_directories() {
    let dir = tempfile::tempdir().expect("临时目录");
    let tool = WriteTool::new();
    let cancel = CancellationToken::new();

    let result = tool
        .invoke(
            json!({"path": "nested/deep/a.txt", "content": "hello"}),
            &ctx(dir.path()),
            &cancel,
        )
        .await;

    assert!(!result.is_error, "{}", result.text_content());
    assert_eq!(
        result.text_content(),
        "Successfully wrote to nested/deep/a.txt",
        "成功文案用原始入参 path，不是绝对路径"
    );
    let written = std::fs::read_to_string(dir.path().join("nested/deep/a.txt")).expect("文件已写");
    assert_eq!(written, "hello");
}

#[tokio::test]
async fn overwrites_an_existing_file() {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::write(dir.path().join("a.txt"), "old").expect("预置文件");
    let tool = WriteTool::new();
    let cancel = CancellationToken::new();

    let result = tool
        .invoke(
            json!({"path": "a.txt", "content": "new"}),
            &ctx(dir.path()),
            &cancel,
        )
        .await;

    assert!(!result.is_error);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("可读"),
        "new"
    );
}

#[tokio::test]
async fn writes_an_empty_file_when_content_is_empty() {
    let dir = tempfile::tempdir().expect("临时目录");
    let tool = WriteTool::new();
    let cancel = CancellationToken::new();

    let result = tool
        .invoke(
            json!({"path": "empty.txt", "content": ""}),
            &ctx(dir.path()),
            &cancel,
        )
        .await;

    assert!(!result.is_error);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("empty.txt")).expect("可读"),
        ""
    );
}

#[tokio::test]
async fn absolute_paths_are_used_as_given() {
    let dir = tempfile::tempdir().expect("临时目录");
    let target = dir.path().join("abs.txt");
    let tool = WriteTool::new();
    let cancel = CancellationToken::new();

    let result = tool
        .invoke(
            json!({"path": target.to_string_lossy(), "content": "x"}),
            &ctx(std::path::Path::new("/nonexistent")),
            &cancel,
        )
        .await;

    assert!(!result.is_error, "{}", result.text_content());
    assert!(target.exists(), "绝对路径不受 cwd 影响");
}

#[tokio::test]
async fn write_into_a_directory_path_reports_an_error() {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::create_dir(dir.path().join("adir")).expect("建目录");
    let tool = WriteTool::new();
    let cancel = CancellationToken::new();

    let result = tool
        .invoke(
            json!({"path": "adir", "content": "x"}),
            &ctx(dir.path()),
            &cancel,
        )
        .await;

    assert!(result.is_error, "写到目录上应当失败");
    assert!(
        result.text_content().contains("Could not write file adir"),
        "错误文本要带路径：{}",
        result.text_content()
    );
}

#[tokio::test]
async fn cancelled_before_write_returns_aborted() {
    let dir = tempfile::tempdir().expect("临时目录");
    let tool = WriteTool::new();
    let cancel = CancellationToken::new();
    cancel.cancel();

    let result = tool
        .invoke(
            json!({"path": "a.txt", "content": "x"}),
            &ctx(dir.path()),
            &cancel,
        )
        .await;

    assert!(result.is_error);
    assert_eq!(result.text_content(), "Operation aborted");
    assert!(!dir.path().join("a.txt").exists(), "取消后不得落盘");
}

#[tokio::test]
async fn missing_required_field_is_caught_before_writing() {
    let dir = tempfile::tempdir().expect("临时目录");
    let tool = WriteTool::new();
    let cancel = CancellationToken::new();

    let result = tool
        .invoke(json!({"path": "a.txt"}), &ctx(dir.path()), &cancel)
        .await;

    assert!(result.is_error);
    assert!(result
        .text_content()
        .contains("must have required properties content"));
    assert!(!dir.path().join("a.txt").exists(), "校验失败不得写文件");
}

#[test]
fn schema_and_description_match_the_recorded_ones() {
    let tool = WriteTool::new();
    assert_eq!(tool.name(), "write");
    assert_eq!(
        tool.schema()["required"],
        json!(["path", "content"]),
        "必填字段与 golden 的 tools 数组一致"
    );
}
