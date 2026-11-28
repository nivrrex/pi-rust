//! P3-13：会话文件的读写与追加。
//!
//! 判据：所有 golden 会话都能整体读入；追加后重新读出保持一致；失败路径有错误。

use pi_core::session::{SessionEntry, SessionHeader};
use pi_core::session_io::{SessionFile, SessionIoError};
use std::path::Path;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

#[test]
fn every_golden_session_reads_back() {
    let golden = workspace().join("golden");
    let mut checked = 0;
    for entry in std::fs::read_dir(&golden).expect("有 golden 目录") {
        let path = entry.expect("可读目录项").path().join("session.jsonl");
        if !path.is_file() {
            continue;
        }
        let session =
            SessionFile::read(&path).unwrap_or_else(|e| panic!("{} 读取失败：{e}", path.display()));
        assert_eq!(session.header.kind, "session");
        assert!(!session.entries.is_empty(), "{} 不该是空的", path.display());
        checked += 1;
    }
    assert!(checked >= 30, "应当检查 30 个以上场景，实际 {checked}");
}

#[test]
fn append_then_read_back_preserves_order_and_content() {
    let dir = tempfile::tempdir().expect("有临时目录");
    let path = dir.path().join("s.jsonl");
    let header = SessionHeader {
        kind: "session".to_string(),
        version: Some(3),
        id: "a1".to_string(),
        timestamp: "2026-01-02T03:04:05.678Z".to_string(),
        cwd: None,
        extra: serde_json::Map::new(),
    };
    let original = SessionFile {
        header: header.clone(),
        entries: Vec::new(),
    };
    original.write(&path).expect("可写入");

    let entry = SessionEntry::ModelChange {
        id: "e1".to_string(),
        parent_id: None,
        timestamp: "2026-01-02T03:04:06.000Z".to_string(),
        provider: "mock".to_string(),
        model_id: "mock-model".to_string(),
        extra: serde_json::Map::new(),
    };
    SessionFile::append(&path, &entry).expect("可追加");

    let read = SessionFile::read(&path).expect("可读回");
    assert_eq!(read.header, header);
    assert_eq!(read.entries, vec![entry]);
}

#[test]
fn append_creates_the_file_when_missing() {
    let dir = tempfile::tempdir().expect("有临时目录");
    let path = dir.path().join("new.jsonl");
    let entry = SessionEntry::ThinkingLevelChange {
        id: "e1".to_string(),
        parent_id: None,
        timestamp: "t".to_string(),
        thinking_level: "high".to_string(),
        extra: serde_json::Map::new(),
    };
    SessionFile::append(&path, &entry).expect("缺文件时应创建");
    assert!(path.is_file());
}

#[test]
fn missing_file_reports_an_io_error_with_the_path() {
    let error = SessionFile::read(Path::new("/nonexistent/definitely-missing.jsonl"))
        .expect_err("不存在的文件应报错");
    match error {
        SessionIoError::Io { path, .. } => {
            assert!(path.to_string_lossy().contains("definitely-missing"));
        }
        other => panic!("应为 Io 错误，实际 {other}"),
    }
}

/// 坏行**跳过而不整份失败**（ref `session-manager.ts:616-624`）：解析失败的行返回 null，
/// 其余行照常加载。此前任一坏行都会让整个会话打不开（真机 `-c` 报"第 6147 行解析失败"）。
#[test]
fn bad_line_is_skipped_and_the_rest_loads() {
    let text = concat!(
        "{\"type\":\"session\",\"version\":3,\"id\":\"a\",\"timestamp\":\"t\"}\n",
        "{\"type\":\"model_change\",\"id\":\"e1\",\"parentId\":null,\"timestamp\":\"t\",\"provider\":\"p\",\"modelId\":\"m\"}\n",
        "not json at all\n",
        "\n",
        "{\"type\":\"model_change\",\"id\":\"e2\",\"parentId\":\"e1\",\"timestamp\":\"t\",\"provider\":\"p\",\"modelId\":\"m\"}\n"
    );
    let file = SessionFile::parse(Path::new("x.jsonl"), text).expect("坏行不应让整份失败");
    assert_eq!(file.entries.len(), 2, "两条合法 entry 都要读出来");
}

#[test]
fn file_without_header_is_rejected() {
    let text = "{\"type\":\"model_change\",\"id\":\"e1\",\"parentId\":null,\"timestamp\":\"t\",\"provider\":\"p\",\"modelId\":\"m\"}\n";
    let error = SessionFile::parse(Path::new("x.jsonl"), text).expect_err("缺 header 应报错");
    assert!(matches!(error, SessionIoError::MissingHeader(_)));
}

#[test]
fn empty_file_is_rejected() {
    let error = SessionFile::parse(Path::new("x.jsonl"), "").expect_err("空文件应报错");
    assert!(matches!(error, SessionIoError::MissingHeader(_)));
}

#[test]
fn round_trip_of_a_golden_session_is_semantically_identical() {
    // 读入 golden 再写出，重新解析后应等价（字段顺序不保证，见 Q8）。
    let path = workspace().join("golden/S01/session.jsonl");
    let session = SessionFile::read(&path).expect("可读");
    let dir = tempfile::tempdir().expect("有临时目录");
    let copy = dir.path().join("copy.jsonl");
    session.write(&copy).expect("可写");
    let again = SessionFile::read(&copy).expect("可再读");
    assert_eq!(session, again, "读→写→读 应语义等价");
}
