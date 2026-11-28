//! `/export` 的 HTML 分支（精简版，见 LIMITATIONS「与上游的差异」）。
//!
//! 契约：把**当前分支**的消息渲染成**单文件、内嵌样式**的 HTML，
//! 用户与助手文本可见、HTML 特殊字符被转义、文件自包含（无外部依赖）。

use std::path::Path;

fn write_session(dir: &Path, name: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let lines = [
        r#"{"type":"session","version":3,"id":"sess-1","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}"#,
        r#"{"type":"message","id":"m1","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"你好 <b>世界</b> & 引号\"x\""}]}}"#,
        r#"{"type":"message","id":"m2","parentId":"m1","timestamp":"2026-01-01T00:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"回答正文"}]}}"#,
    ];
    std::fs::write(&path, lines.join("\n") + "\n").expect("写会话");
    path
}

#[test]
fn html_export_is_self_contained_and_escaped() {
    let tmp = tempfile::tempdir().expect("tmp");
    let source = write_session(tmp.path(), "session.jsonl");
    let target = tmp.path().join("out.html");
    let produced = pi_cli::tui::export_branch_to_html(&source, &target).expect("应导出");
    let html = std::fs::read_to_string(&produced).expect("读回");

    assert!(html.starts_with("<!DOCTYPE html>"), "应是完整 HTML 文档");
    assert!(html.contains("<style>"), "样式必须内嵌");
    assert!(!html.contains("<link "), "不得引用外部样式");
    assert!(!html.contains("<script src"), "不得引用外部脚本");

    // 正文可见。
    assert!(html.contains("回答正文"), "助手文本应出现：{html}");
    assert!(html.contains("你好"), "用户文本应出现");
    // HTML 特殊字符被转义，不能原样进标签。
    assert!(html.contains("&lt;b&gt;世界&lt;/b&gt;"), "尖括号应转义");
    assert!(html.contains("&amp;"), "& 应转义");
}

/// 默认文件名：`pi-session-<源文件名>.html`（ref `export-html/index.ts:275-277`）。
#[test]
fn html_export_default_name_is_derived_from_session() {
    let tmp = tempfile::tempdir().expect("tmp");
    let source = write_session(tmp.path(), "session.jsonl");
    let produced = pi_cli::tui::export_branch_to_html(&source, Path::new("")).expect("应导出");
    let name = produced
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        name.starts_with("pi-session-") && name.ends_with(".html"),
        "默认文件名不符：{name}"
    );
    let _ = std::fs::remove_file(&produced);
}

/// content 为空数组的 assistant（被中断的回合）**跳过**，不生成空段。
///
/// 真机遇到：mock 回复被打断，会话里留下 `content: []` 的 assistant，
/// 导出结果只剩用户那句——那不是缺陷，是正确跳过。
#[test]
fn empty_content_messages_are_skipped() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("empty.jsonl");
    let lines = [
        r#"{"type":"session","version":3,"id":"s","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}"#,
        r#"{"type":"message","id":"a","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"只有这句"}]}}"#,
        r#"{"type":"message","id":"b","parentId":"a","timestamp":"2026-01-01T00:00:02.000Z","message":{"role":"assistant","content":[]}}"#,
    ];
    std::fs::write(&path, lines.join("\n") + "\n").expect("写会话");
    let target = tmp.path().join("out.html");
    let produced = pi_cli::tui::export_branch_to_html(&path, &target).expect("应导出");
    let html = std::fs::read_to_string(&produced).expect("读回");
    assert_eq!(
        html.matches("class=\"msg ").count(),
        1,
        "只应有 user 一段：{html}"
    );
    assert!(html.contains("只有这句"), "用户那句应在：{html}");
}
