//! F52：`edit` 的 `prepareArguments`（ref `packages/agent/src/harness/tools/edit.ts:55-75`）。
//!
//! ref 在**校验之前**归一参数，三种形态都接受：
//! 1. `edits` 是 JSON 字符串（模型把数组当字符串发）→ `JSON.parse`，数组或单个 edit 都行（`:56-66`）；
//! 2. `edits` 是单个 edit 对象 → 包成数组（`:67-69`）；
//! 3. 顶层 `oldText`/`newText`（legacy 单次替换）→ 追加进 `edits`，并删掉两个顶层字段（`:71-75`）。
//!
//! 我们此前只认标准形态，另两种会走到必填校验，回给模型
//! `Validation failed for tool "edit"`——TS 不会产出这条错误。

use pi_core::tools::edit::EditTool;
use pi_core::tools::tool::{Tool, ToolContext};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn ctx(dir: &std::path::Path) -> ToolContext {
    ToolContext {
        cwd: dir.to_path_buf(),
    }
}

#[tokio::test]
async fn legacy_top_level_old_text_is_folded_into_edits() {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").expect("预置文件");
    let result = EditTool
        .invoke(
            json!({"path": "a.txt", "oldText": "one", "newText": "ONE"}),
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
async fn single_edit_object_is_wrapped_into_an_array() {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").expect("预置文件");
    let result = EditTool
        .invoke(
            json!({"path": "a.txt", "edits": {"oldText": "two", "newText": "TWO"}}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error, "{}", result.text_content());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("可读"),
        "one\nTWO\n"
    );
}

#[tokio::test]
async fn edits_sent_as_a_json_string_is_parsed() {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").expect("预置文件");
    let result = EditTool
        .invoke(
            json!({
                "path": "a.txt",
                "edits": r#"[{"oldText":"one","newText":"ONE"}]"#,
            }),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error, "{}", result.text_content());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("可读"),
        "ONE\ntwo\n"
    );
}
