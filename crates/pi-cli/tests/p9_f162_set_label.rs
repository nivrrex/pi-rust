//! `pi.setLabel(entryId, label)`（ref `agent-session.ts:3079-3081` →
//! `session-manager.ts:1439-1460` 的 `appendLabelChange`）。
//!
//! 契约（三条都来自 ref）：
//! 1. 写一条 `{type:"label", id, parentId, timestamp, targetId, label}` 条目；
//! 2. **目标条目必须存在**，否则报 `Entry <id> not found`；
//! 3. `label` 为 `undefined`/空 ⇒ 写 `label: null`（表示清除标记）。
//!
//! 此前 pi-rs 没有该 API，扩展调用会拿到 `TypeError`（runner 上没有这个方法）。

use std::path::Path;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn session_with_entries(dir: &Path, lines: &[&str]) -> std::path::PathBuf {
    let path = dir.join("session.jsonl");
    let mut text = String::from(
        "{\"type\":\"session\",\"version\":3,\"id\":\"seed\",\"timestamp\":\"t0\",\"cwd\":\"/tmp\"}\n",
    );
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(&path, text).expect("写会话");
    path
}

#[test]
fn label_entry_has_the_ts_shape() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = session_with_entries(
        tmp.path(),
        &["{\"type\":\"message\",\"id\":\"m1\",\"parentId\":null,\"timestamp\":\"t1\",\"message\":{\"role\":\"user\",\"content\":[]}}"],
    );
    let mut writer = pi_cli::session_writer::SessionWriter::open(&path).expect("打开会话");
    writer
        .append_label_change("m1", Some("重要"))
        .expect("写 label");
    let text = std::fs::read_to_string(&path).expect("读回");
    let last = text.lines().last().expect("最后一行");
    let value: serde_json::Value = serde_json::from_str(last).expect("合法 JSON");
    assert_eq!(value["type"], "label", "{value}");
    assert_eq!(value["targetId"], "m1", "{value}");
    assert_eq!(value["label"], "重要", "{value}");
    // `parentId` 是写入时的叶子（这里是 m1），`id`/`timestamp` 必须存在。
    assert_eq!(value["parentId"], "m1", "{value}");
    assert!(value["id"].is_string(), "{value}");
    assert!(value["timestamp"].is_string(), "{value}");
    let _ = repo_root();
}

/// 目标条目不存在 ⇒ 报错，**不写任何条目**（ref 在构造 entry 前就 throw）。
#[test]
fn unknown_target_is_rejected_without_writing() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = session_with_entries(
        tmp.path(),
        &["{\"type\":\"message\",\"id\":\"m1\",\"parentId\":null,\"timestamp\":\"t1\",\"message\":{\"role\":\"user\",\"content\":[]}}"],
    );
    let before = std::fs::read_to_string(&path).expect("读回");
    let mut writer = pi_cli::session_writer::SessionWriter::open(&path).expect("打开会话");
    let error = writer
        .append_label_change("nope", Some("x"))
        .expect_err("应报错");
    assert!(error.contains("nope"), "错误里应提到目标 id：{error}");
    let after = std::fs::read_to_string(&path).expect("读回");
    assert_eq!(before, after, "不应写入任何条目");
}

/// 空标记 ⇒ `label: null`（ref 用 undefined 表示清除）。
#[test]
fn empty_label_clears_the_mark() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = session_with_entries(
        tmp.path(),
        &["{\"type\":\"message\",\"id\":\"m1\",\"parentId\":null,\"timestamp\":\"t1\",\"message\":{\"role\":\"user\",\"content\":[]}}"],
    );
    let mut writer = pi_cli::session_writer::SessionWriter::open(&path).expect("打开会话");
    writer.append_label_change("m1", None).expect("写 label");
    let text = std::fs::read_to_string(&path).expect("读回");
    let last = text.lines().last().expect("最后一行");
    let value: serde_json::Value = serde_json::from_str(last).expect("合法 JSON");
    assert_eq!(value["label"], serde_json::Value::Null, "{value}");
}
