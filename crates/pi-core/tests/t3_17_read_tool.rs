//! P3.1：`read` 工具——文本（含截断/offset/limit）、图片、错误路径、取消。

use pi_core::tools::read::{base64_encode, detect_image_mime_type, ReadTool};
use pi_core::tools::tool::{Tool, ToolContext, ToolResultBlock};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn ctx(dir: &std::path::Path) -> ToolContext {
    ToolContext {
        cwd: dir.to_path_buf(),
    }
}

#[tokio::test]
async fn reads_a_text_file() {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::write(dir.path().join("a.txt"), "line1\nline2\n").expect("预置");
    let result = ReadTool
        .invoke(
            json!({"path": "a.txt"}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error, "{}", result.text_content());
    // **尾随换行必须保留**：golden/S02 里 TS 读到 `alpha\nbeta\n` 时工具结果就是
    // `alpha\nbeta\n`。我最初这里写的是 `line1\nline2`（少了尾换行），是把当时的
    // 实现行为当成了期望——由 `t3_26_loop_scenarios` 的场景重放抓出来。
    assert_eq!(result.text_content(), "line1\nline2\n");
}

#[tokio::test]
async fn missing_file_reports_the_node_style_error() {
    let dir = tempfile::tempdir().expect("临时目录");
    let result = ReadTool
        .invoke(
            json!({"path": "nope.txt"}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error);
    let text = result.text_content();
    assert!(
        text.starts_with("ENOENT: no such file or directory, access '"),
        "{text}"
    );
    assert!(text.contains("nope.txt"), "错误里要带绝对路径：{text}");
}

#[tokio::test]
async fn offset_beyond_end_reports_the_typescript_text() {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::write(dir.path().join("a.txt"), "a\nb").expect("预置");
    let result = ReadTool
        .invoke(
            json!({"path": "a.txt", "offset": 9}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error);
    assert_eq!(
        result.text_content(),
        "Offset 9 is beyond end of file (2 lines total)"
    );
}

#[tokio::test]
async fn limit_and_truncation_hints_are_applied() {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::write(dir.path().join("a.txt"), "a\nb\nc\nd").expect("预置");
    let result = ReadTool
        .invoke(
            json!({"path": "a.txt", "limit": 2}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(
        result.text_content(),
        "a\nb\n\n[2 more lines in file. Use offset=3 to continue.]"
    );
}

#[tokio::test]
async fn image_files_produce_text_plus_image_blocks() {
    let dir = tempfile::tempdir().expect("临时目录");
    // 1x1 PNG 的最小字节（内容不重要，工具只按扩展名识别）。
    let png: Vec<u8> = vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
    ];
    std::fs::write(dir.path().join("pic.png"), &png).expect("预置");

    let result = ReadTool
        .invoke(
            json!({"path": "pic.png"}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;

    assert!(!result.is_error);
    assert_eq!(result.content.len(), 2, "文本块 + 图片块");
    assert_eq!(
        result.content[0],
        ToolResultBlock::Text {
            text: "Read image file [image/png]".to_string()
        }
    );
    match &result.content[1] {
        ToolResultBlock::Image { data, mime_type } => {
            assert_eq!(mime_type, "image/png");
            assert_eq!(data, &base64_encode(&png));
        }
        other => panic!("第二个块应是图片，实际 {other:?}"),
    }
}

#[tokio::test]
async fn cancelled_read_does_not_touch_the_file() {
    let dir = tempfile::tempdir().expect("临时目录");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = ReadTool
        .invoke(json!({"path": "a.txt"}), &ctx(dir.path()), &cancel)
        .await;
    assert!(result.is_error);
    assert_eq!(result.text_content(), "Operation aborted");
}

#[test]
fn image_extension_detection_covers_the_documented_formats() {
    for (name, mime) in [
        ("a.jpg", "image/jpeg"),
        ("a.jpeg", "image/jpeg"),
        ("a.PNG", "image/png"),
        ("a.gif", "image/gif"),
        ("a.webp", "image/webp"),
        ("a.bmp", "image/bmp"),
    ] {
        assert_eq!(
            detect_image_mime_type(std::path::Path::new(name)),
            Some(mime),
            "{name}"
        );
    }
    assert_eq!(detect_image_mime_type(std::path::Path::new("a.txt")), None);
    assert_eq!(detect_image_mime_type(std::path::Path::new("noext")), None);
}

#[test]
fn base64_encoding_matches_known_vectors() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
}

#[test]
fn schema_matches_the_recorded_tools_array() {
    assert_eq!(ReadTool.schema()["required"], json!(["path"]));
    assert_eq!(
        ReadTool.schema()["properties"]["offset"]["type"],
        "number",
        "offset 是可选数字"
    );
}
