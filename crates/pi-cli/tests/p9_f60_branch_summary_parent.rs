//! F60/13：`branch_summary` 条目的 `parentId` 必须是**导航目标**，
//! `fromId` 是**被离开的旧叶子**（ref `session-manager.ts:1593-1618` 的 `branchWithSummary`）。
//!
//! ref 的实现顺序是：
//! ```text
//! const fromId = this.leafId ?? "root";   // 旧叶子
//! this.leafId = branchFromId;             // 叶子先挪到目标
//! entry = { parentId: branchFromId, fromId, summary, … };
//! ```
//!
//! pi-rs 此前用 `take_id_and_parent()`（`parentId` = 当前叶子 = **旧分支末尾**）⇒
//! 摘要被挂到旧分支上：会话树里旧分支多出一条子节点，而**切过去的分支上什么都没有**，
//! 于是 `getBranch()` 与新屏幕不一致（`pi-handoff-gate` 从 `getBranch()` 重建清单，
//! 会拿到错的分支）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_cli::session_writer::SessionWriter;
use std::path::Path;

/// 建一份会话并返回写入器与它写出的最后一条消息 id。
fn prepare(dir: &Path) -> (SessionWriter, String) {
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&sessions).expect("建目录");
    let mut writer = SessionWriter::create(&sessions, dir, "p", "m", "off").expect("建会话");
    // 造一条真实消息，作为"旧叶子"。
    writer
        .append_message(&pi_ai::message::Message::User {
            content: pi_ai::UserContent::Text("旧分支的问题".to_string()),
            timestamp: 1,
            extra: serde_json::Map::new(),
        })
        .expect("写消息");
    // 会话文件是**懒创建**的（ref 同语义：第一条 assistant 之后才落盘），
    // 所以"旧叶子 id"不能从文件读。改从**链**上取——`append_message` 已把它写进
    // `chain`（`take_id_and_parent` 分配 id 后回写），这正是生产路径里
    // `append_branch_summary` 看到的同一个值。
    let chain = std::sync::Arc::new(std::sync::Mutex::new(None));
    writer.set_chain(std::sync::Arc::clone(&chain));
    writer
        .append_message(&pi_ai::message::Message::User {
            content: pi_ai::UserContent::Text("再一条".to_string()),
            timestamp: 2,
            extra: serde_json::Map::new(),
        })
        .expect("写第二条消息");
    let old_leaf = chain
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .expect("链上应有叶子");
    // **触发首次落盘**：ref 在第一条 assistant 消息出现时才建文件
    // （`session-manager.ts:1167-1184`），pi-rs 同语义（`write_first_flush`）。
    // 不写它就只能在内存里验证，读不到文件。
    writer
        .append_message(&pi_ai::message::Message::Assistant(Box::new(
            pi_ai::types::AssistantMessage {
                role: "assistant".to_string(),
                content: vec![pi_ai::types::Content::Text {
                    text: "旧分支的回答".to_string(),
                    text_signature: None,
                }],
                api: "openai-completions".to_string(),
                provider: "p".to_string(),
                model: "m".to_string(),
                response_model: None,
                usage: pi_ai::types::Usage::default(),
                stop_reason: pi_ai::types::StopReason::Stop,
                error_message: None,
                raw_stop_reason: None,
                end_turn: None,
                response_id: None,
                timestamp: 3,
            },
        )))
        .expect("写 assistant 触发落盘");
    (writer, old_leaf)
}

/// `parentId` = 导航目标；`fromId` = 旧叶子；且链上的叶子变成新摘要条目。
#[test]
fn branch_summary_hangs_off_the_navigation_target() {
    let dir = tempfile::tempdir().expect("临时目录");
    let (mut writer, old_leaf) = prepare(dir.path());
    // 导航目标取头节点（模拟"回到更早的位置"）。
    let header_id = writer.header_id().expect("header");
    let target = header_id.clone();

    writer
        .append_branch_summary(Some(&target), &old_leaf, "分支摘要", 10, 5)
        .expect("写摘要");

    let text = std::fs::read_to_string(writer.path()).expect("读会话");
    let summary = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value.get("type").and_then(|v| v.as_str()) == Some("branch_summary"))
        .expect("应写出 branch_summary");
    assert_eq!(
        summary.get("parentId").and_then(|v| v.as_str()),
        Some(target.as_str()),
        "parentId 应是**导航目标**（ref `parentId: branchFromId`），实际 {:?}",
        summary.get("parentId")
    );
    assert_eq!(
        summary.get("fromId").and_then(|v| v.as_str()),
        Some(old_leaf.as_str()),
        "fromId 应是**被离开的旧叶子**（ref `const fromId = this.leafId`），实际 {:?}",
        summary.get("fromId")
    );
    assert_ne!(
        summary.get("parentId"),
        summary.get("fromId"),
        "parentId 与 fromId 不能相同（旧实现把 parent 也设成旧叶子）"
    );
}

/// 导航到根：`parentId` 为 `null`（ref 允许 `branchFromId === null`）。
#[test]
fn branch_summary_to_root_has_null_parent() {
    let dir = tempfile::tempdir().expect("临时目录");
    let (mut writer, old_leaf) = prepare(dir.path());
    writer
        .append_branch_summary(None, &old_leaf, "回到开头", 1, 1)
        .expect("写摘要");
    let text = std::fs::read_to_string(writer.path()).expect("读会话");
    let summary = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value.get("type").and_then(|v| v.as_str()) == Some("branch_summary"))
        .expect("应写出 branch_summary");
    assert!(
        summary
            .get("parentId")
            .is_some_and(serde_json::Value::is_null),
        "导航到根时 parentId 应为 null，实际 {:?}",
        summary.get("parentId")
    );
}
