//! P4.0a 的验收测试：会话里的 **system 条目形状**与 rpc/print 同源。
//!
//! 背景（挂起项 2）：rpc 路径此前只传 `--system-prompt` 开关值（默认为空），
//! 既不组装系统提示、也不写 system 条目，且写出来的是**空 sections** 的条目。
//! 表现是 rpc 场景（S09a/S09b/S10/S11）的会话文件恒少 1 条。

use std::path::Path;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pi-rs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// **ref 语义**：会话文件在第一条 assistant 消息出现前**不存在**
/// （`session-manager.ts:1160-1190` 的 `_persist`：没有 assistant 就只攒在内存里）。
/// 本文件的两个用例断言的是"system 条目形状"，因此写完后补一条 assistant 消息触发落盘。
fn append_assistant(writer: &mut pi_cli::session_writer::SessionWriter) {
    let message = pi_ai::types::AssistantMessage {
        role: "assistant".to_string(),
        content: Vec::new(),
        api: "openai-completions".to_string(),
        provider: "mock".to_string(),
        model: "m1".to_string(),
        response_model: None,
        usage: pi_ai::types::Usage::default(),
        stop_reason: pi_ai::types::StopReason::Stop,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 1,
    };
    writer
        .append_message(&pi_ai::message::Message::Assistant(Box::new(message)))
        .expect("写得进去");
}

fn read_entries(dir: &Path) -> Vec<serde_json::Value> {
    // 会话文件可能在 `dir` 下或 `dir/sessions` 下（两种布局都兼容）。
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    for root in [dir.to_path_buf(), dir.join("sessions")] {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.filter_map(|entry| entry.ok()) {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "jsonl") {
                    candidates.push(path);
                }
            }
        }
    }
    let file = candidates.into_iter().min().expect("有会话文件");
    std::fs::read_to_string(file)
        .expect("读得到会话文件")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("每行是 JSON"))
        .collect()
}

/// system 条目必须带 `sections` 与四键 `toolsAdded`（与 golden/S09a 第 4 行同形状）。
#[test]
fn system_entry_carries_sections_and_tools_added() {
    let dir = temp_dir("system-entry");
    let mut writer = pi_cli::session_writer::SessionWriter::create(
        &dir,
        Path::new("/tmp/x"),
        "mock",
        "m1",
        "off",
    )
    .expect("建得起来");
    let sections = vec![
        ("preamble".to_string(), "你好".to_string()),
        ("cwd".to_string(), "<cwd>\n/tmp/x\n</cwd>".to_string()),
    ];
    let tools = vec![serde_json::json!({
        "name": "read",
        "description": "读文件",
        "parameters": { "type": "object" },
        "constrainedSampling": { "type": "json_schema", "strict": "prefer" },
    })];
    writer
        .append_system_message(&sections, &tools, "")
        .expect("写得进去");
    append_assistant(&mut writer);

    let entries = read_entries(&dir);
    let system = entries
        .iter()
        .find(|entry| entry["type"] == "message" && entry["message"]["role"] == "system")
        .expect("有 system 条目");
    let message = &system["message"];
    assert_eq!(message["content"], "", "正文是空串，内容由 sections 承载");
    assert_eq!(message["sections"]["preamble"], "你好");
    assert_eq!(message["sections"]["cwd"], "<cwd>\n/tmp/x\n</cwd>");
    let added = message["toolsAdded"].as_array().expect("toolsAdded 是数组");
    assert_eq!(added.len(), 1);
    // **四键**：我此前只写了 name/description，逐行核对立刻显出差异。
    for key in ["name", "description", "parameters", "constrainedSampling"] {
        assert!(added[0].get(key).is_some(), "toolsAdded[0] 必须有 {key}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 空 sections 时**不写** `sections` 键（避免再用空 sections 冒充"已组装"）。
#[test]
fn empty_sections_write_no_sections_key() {
    let dir = temp_dir("system-entry-empty");
    let mut writer = pi_cli::session_writer::SessionWriter::create(
        &dir,
        Path::new("/tmp/x"),
        "mock",
        "m1",
        "off",
    )
    .expect("建得起来");
    writer
        .append_system_message(&[], &[], "")
        .expect("写得进去");
    append_assistant(&mut writer);
    let entries = read_entries(&dir);
    let system = entries
        .iter()
        .find(|entry| entry["type"] == "message" && entry["message"]["role"] == "system")
        .expect("有 system 条目");
    assert!(
        system["message"].get("sections").is_none(),
        "空 sections 不应写入该键"
    );
    assert!(
        system["message"].get("toolsAdded").is_none(),
        "空工具列表不应写入该键"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
