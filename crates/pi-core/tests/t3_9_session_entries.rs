//! P3-11：session entry 的解析与保真。
//!
//! 判据：所有 golden 会话文件（33 个场景）都必须能解析；未知类型原样保留。

use pi_core::session::{parse_line, SessionEntry, SessionLine};
use serde_json::Value;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn session_files() -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(workspace().join("golden"))
        .expect("有 golden 目录")
        .filter_map(|entry| entry.ok().map(|e| e.path().join("session.jsonl")))
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files
}

#[test]
fn every_golden_session_parses() {
    let files = session_files();
    assert!(
        files.len() >= 30,
        "应当有 30 个以上场景，实际 {}",
        files.len()
    );
    let mut kinds: Vec<String> = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("可读");
        for (index, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
            let parsed = parse_line(line)
                .unwrap_or_else(|e| panic!("{} 第 {} 行解析失败：{e}", path.display(), index + 1));
            let kind = match &parsed {
                SessionLine::Header(_) => "session".to_string(),
                SessionLine::Entry(entry) => entry.kind().to_string(),
            };
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
    }
    kinds.sort();
    assert_eq!(
        kinds,
        vec![
            // S27 的种子带这两类；`convertToLlm`（`packages/coding-agent/src/core/messages.ts:163-176`）
            // 把它们变成 **user** 消息，所以它们也要能解析并进上下文。
            "branch_summary",
            "compaction",
            "context_edit",
            "custom_message",
            "message",
            "model_change",
            "session",
            "thinking_level_change"
        ],
        "golden 里出现的 entry 类型都要能解析"
    );
}

#[test]
fn header_fields_survive_a_round_trip() {
    let line = std::fs::read_to_string(workspace().join("golden/S01/session.jsonl")).expect("可读");
    let first = line.lines().next().expect("有首行");
    let SessionLine::Header(header) = parse_line(first).expect("可解析") else {
        panic!("首行应是 header");
    };
    assert_eq!(header.kind, "session");
    assert_eq!(header.version, Some(3), "当前版本是 3");
    assert!(!header.id.is_empty());
    let written = serde_json::to_value(&header).expect("可写回");
    let original: Value = serde_json::from_str(first).expect("原文是 JSON");
    assert_eq!(written, original, "header 读后写回要一致");
}

#[test]
fn unknown_entry_types_are_preserved_verbatim() {
    let line = r#"{"type":"future_thing","id":"e1","parentId":null,"timestamp":"2026-01-01T00:00:00.000Z","payload":{"deep":[1,2,3]}}"#;
    let SessionLine::Entry(entry) = parse_line(line).expect("可解析") else {
        panic!("应是 entry");
    };
    assert_eq!(entry.kind(), "future_thing");
    assert_eq!(entry.id(), Some("e1"));
    let written = serde_json::to_value(&entry).expect("可写回");
    let original: Value = serde_json::from_str(line).expect("原文是 JSON");
    assert_eq!(written, original, "未知类型必须原样保留");
}

#[test]
fn message_entries_carry_a_typed_message() {
    let text = std::fs::read_to_string(workspace().join("golden/S25-anthropic/session.jsonl"))
        .expect("可读");
    let mut roles: Vec<&str> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let SessionLine::Entry(SessionEntry::Message { message, .. }) =
            parse_line(line).expect("可解析")
        else {
            continue;
        };
        if !roles.contains(&message.role()) {
            roles.push(message.role());
        }
    }
    roles.sort_unstable();
    assert_eq!(roles, vec!["assistant", "system", "toolResult", "user"]);
}

#[test]
fn parent_id_is_preserved_and_optionally_absent() {
    let line = r#"{"type":"model_change","id":"m1","parentId":null,"timestamp":"t","provider":"mock","modelId":"x"}"#;
    let SessionLine::Entry(entry) = parse_line(line).expect("可解析") else {
        panic!("应是 entry");
    };
    assert_eq!(entry.parent_id(), None, "显式 null 视为无父");
}
