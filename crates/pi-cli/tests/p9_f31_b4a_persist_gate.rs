//! F31-B/B4a：落盘门控（ref `core/session-manager.ts:1160-1185`）。
//!
//! ref 语义：**没有 assistant 消息时一个字节都不写**（system / bashExecution 只进内存），
//! 首条 assistant 到达时把累积条目**按顺序整体写出**，之后逐条追加。
//! 我们此前在 `!` 命令路径上 `ensure_flushed()` 强制落盘，导致 bash-only 会话也留下 jsonl
//! 并打印 `To resume this session`（TS 实测不会）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use pi_ai::message::{Content, Message};
use pi_cli::session_writer::SessionWriter;

fn count_jsonl(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
                .count()
        })
        .unwrap_or(0)
}

fn bash_message(command: &str) -> Message {
    Message::BashExecution {
        command: command.to_string(),
        output: format!("{command}\n"),
        exit_code: Some(0),
        truncated: false,
        full_output_path: None,
        cancelled: false,
        timestamp: 0,
        exclude_from_context: Some(false),
        extra: serde_json::Map::new(),
    }
}

fn user_message(text: &str) -> Message {
    Message::User {
        content: pi_ai::message::UserContent::Blocks(vec![Content::Text {
            text: text.to_string(),
            text_signature: None,
        }]),
        timestamp: 0,
        extra: serde_json::Map::new(),
    }
}

fn assistant_message(text: &str) -> Message {
    Message::Assistant(Box::new(pi_ai::AssistantMessage {
        role: "assistant".to_string(),
        content: vec![Content::Text {
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

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().expect("临时目录");
    let sessions = tmp.path().join("sessions");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&sessions).expect("建会话目录");
    std::fs::create_dir_all(&work).expect("建工作目录");
    (tmp, sessions, work)
}

#[test]
fn bash_only_session_creates_no_file() {
    let (_tmp, sessions, work) = setup();
    let mut writer =
        SessionWriter::create(&sessions, &work, "mock", "mock-model", "off").expect("建会话");
    writer
        .append_system_message(&[], &[], "")
        .expect("写 system");
    writer
        .append_message(&bash_message("echo ONLY-BASH"))
        .expect("写 bash");
    // `!` 命令路径会显式调这个（见 `crates/pi-cli/src/tui.rs`）。
    writer.ensure_flushed().expect("ensure_flushed");
    assert_eq!(
        count_jsonl(&sessions),
        0,
        "ref：没有 assistant 消息的会话不落盘（`session-manager.ts:1163-1175`）"
    );
}

#[test]
fn first_assistant_flushes_everything_in_order() {
    let (_tmp, sessions, work) = setup();
    let mut writer =
        SessionWriter::create(&sessions, &work, "mock", "mock-model", "off").expect("建会话");
    let path = writer.path().to_path_buf();
    writer
        .append_system_message(&[], &[], "")
        .expect("写 system");
    writer.append_message(&user_message("hi")).expect("写 user");
    writer
        .append_message(&bash_message("echo AAA"))
        .expect("写 bash");
    assert_eq!(count_jsonl(&sessions), 0, "assistant 到达前不建文件");
    writer
        .append_message(&assistant_message("Hello world"))
        .expect("写 assistant");
    assert_eq!(count_jsonl(&sessions), 1, "首条 assistant 触发落盘");
    let text = std::fs::read_to_string(&path).expect("读会话文件");
    let roles: Vec<String> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| {
            let message = value.get("message")?;
            message
                .get("role")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect();
    assert_eq!(
        roles,
        vec!["system", "user", "bashExecution", "assistant"],
        "条目必须按内存顺序整体写出（`session-manager.ts:1177-1184`）"
    );
}
