//! S07/S08 恢复前的**最小验证**：会话文件的内容只取决于**消息序列**，与"何时写入"无关。
//!
//! 背景：TS 的重试要在第二次 `agent_start` **之前**发 `entry_appended`，它的 `targetId` 指向
//! **失败 assistant 的会话 entry**——即 TS 在 run 期间就已落盘。我这边的 CLI 是
//! `run.execute()` 返回后才逐条 append，重试时那条 entry 还不存在（见 notes/blocked-S07.md）。
//!
//! 要动"落盘时机"就得先确认：**早写与晚写产出的文件是否一致**。本测试验证的是其中可离线验证的
//! 那一半——写入器对同一序列是确定性的（id/parentId 链只由 append 顺序决定）。
//! 另一半（run 期间的**消息序列**与 run 之后是否相同）要等增量落盘真的实现后，由 18 场景的
//! A 层回归覆盖；此处不假装验证了它。

use pi_ai::message::Message;
use pi_cli::session_writer::SessionWriter;

fn user(text: &str) -> Message {
    Message::User {
        content: pi_ai::UserContent::Text(text.to_string()),
        timestamp: 0,
        extra: serde_json::Map::new(),
    }
}

fn assistant(text: &str) -> Message {
    Message::Assistant(Box::new(pi_ai::AssistantMessage {
        role: "assistant".to_string(),
        content: vec![pi_ai::Content::Text {
            text: text.to_string(),
            text_signature: None,
        }],
        api: "openai-completions".to_string(),
        provider: "mock".to_string(),
        model: "mock-model".to_string(),
        response_model: None,
        usage: pi_ai::Usage::default(),
        stop_reason: pi_ai::StopReason::Stop,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 0,
    }))
}

/// 把 id 与时间戳换成占位符（它们每次都不同，且与"何时写"无关）。
fn normalize(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        for key in ["id", "parentId", "targetId", "timestamp"] {
            if value.get(key).is_some() {
                let placeholder = if value[key].is_null() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(format!("<{key}>"))
                };
                value[key] = placeholder;
            }
        }
        out.push_str(&serde_json::to_string(&value).unwrap_or_default());
        out.push('\n');
    }
    out
}

#[test]
fn the_session_file_depends_only_on_the_message_sequence() {
    let messages = vec![user("first"), assistant("hello"), user("second")];

    // 两边用**同一个 cwd**，否则 header 里的 `cwd` 不同会掩盖真正要比的东西。
    let base = tempfile::tempdir().expect("基础临时目录");
    let cwd = base.path().join("work");
    std::fs::create_dir_all(&cwd).expect("建 work");

    // 路径 A：跑完一次性写（当前 CLI 的做法）。
    let dir_a = base.path().join("a");
    let mut writer_a =
        SessionWriter::create(&dir_a, &cwd, "mock", "mock-model", "off").expect("建会话");
    for message in &messages {
        writer_a.append_message(message).expect("写消息");
    }
    let file_a = std::fs::read_to_string(writer_a.path()).expect("读会话 A");

    // 路径 B：模拟"每产生一条就写"——同样的序列，只是调用时机不同。
    let dir_b = base.path().join("b");
    let mut writer_b =
        SessionWriter::create(&dir_b, &cwd, "mock", "mock-model", "off").expect("建会话");
    for message in &messages {
        writer_b.append_message(message).expect("写消息");
    }
    let file_b = std::fs::read_to_string(writer_b.path()).expect("读会话 B");

    assert_eq!(
        normalize(&file_a),
        normalize(&file_b),
        "同一消息序列，无论何时写入，条目链都应一致"
    );

    // 负向对照：少一条消息必须让文件不同——否则上面那条断言可能因为"两边都是空"而假通过。
    let dir_c = base.path().join("c");
    let mut writer_c =
        SessionWriter::create(&dir_c, &cwd, "mock", "mock-model", "off").expect("建会话");
    for message in &messages[..2] {
        writer_c.append_message(message).expect("写消息");
    }
    let file_c = std::fs::read_to_string(writer_c.path()).expect("读会话 C");
    assert_ne!(
        normalize(&file_a),
        normalize(&file_c),
        "少一条消息必须产出不同的文件（负向对照）"
    );
}

#[test]
fn context_edit_targets_the_previous_entry_and_carries_null_replacement() {
    let dir = tempfile::tempdir().expect("临时目录");
    let mut writer = SessionWriter::create(
        &dir.path().join("s"),
        dir.path(),
        "mock",
        "mock-model",
        "off",
    )
    .expect("建会话");
    writer.append_message(&user("first")).expect("写消息");
    let target = writer.last_entry_id().expect("有上一条 id").to_string();
    writer.append_message(&assistant("failed")).expect("写消息");
    let failed_id = writer.last_entry_id().expect("有上一条 id").to_string();
    writer
        .append_context_edit(&failed_id)
        .expect("写 context_edit");

    let text = std::fs::read_to_string(writer.path()).expect("读会话");
    let entries: Vec<serde_json::Value> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("每行是 JSON"))
        .collect();
    let last = entries.last().expect("有最后一条");
    assert_eq!(last["type"], "context_edit", "最后一条应是 context_edit");
    assert_eq!(last["targetId"], failed_id, "targetId 应指向失败的那条");
    assert!(
        last["replacement"].is_null(),
        "replacement 应为 null（表示移除）"
    );
    assert_eq!(last["parentId"], failed_id, "parentId 应指向上一条");
    // 负向对照：`target_id` 若不是上一条，`parentId` 与它不该混同。
    assert_ne!(target, failed_id, "两次 append 的 id 必须不同");
}
