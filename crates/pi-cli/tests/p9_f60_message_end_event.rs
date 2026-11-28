//! F60/18：`message_end` 必须派发给扩展（ref `agent-session.ts:1093-1099`）。
//!
//! 已装扩展有 4 处订阅它：`pi-agent-resilience`（复位看门狗/预算）、
//! `pi-ai-summary`（累计 token 与费用）、`context-gateway`（响应遥测）。
//! 此前 pi-rs **完全不派发** ⇒ 这些逻辑从未执行。
//!
//! 本用例走真实 `TuiEventSink`（生产路径），断言助手的定稿消息带 `role` 到达扩展。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn install(agent: &Path) {
    let target = agent.join("extensions/message-end-probe");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body =
        std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/message-end-ext.mjs"))
            .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 三条断言必须在**同一个用例**里：`PI_EXT_TEST_LOG` 是进程级环境变量，
/// 多个 `#[test]` 并行会互相覆盖（F60/17 踩过）。
#[test]
fn message_end_reaches_extensions() {
    let Some(_node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install(agent.path());
    let log = agent.path().join("probe.log");
    // 追加写：先删旧日志，否则"改前应红"会假通过（F60/17 踩过）。
    let _ = std::fs::remove_file(&log);
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("node"));
    std::env::set_var("PI_EXT_TEST_LOG", &log);
    pi_cli::ext_tools::configure_ctx(agent.path(), "tui", true);
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000d2".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();

    let (tx, _rx) = std::sync::mpsc::channel();
    let sink = pi_cli::tui::TuiEventSink::new(
        tx,
        200_000,
        pi_ai::message::Message::System {
            content: pi_ai::UserContent::Text(String::new()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        },
    );
    use pi_core::agent::EventSink as _;
    sink.emit(pi_core::agent::AgentEvent::MessageEnd {
        message: pi_ai::message::Message::Assistant(Box::new(pi_ai::types::AssistantMessage {
            role: "assistant".to_string(),
            content: vec![pi_ai::types::Content::Text {
                text: "回答".to_string(),
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
            timestamp: 0,
        })),
    });

    let body = {
        let mut text = String::new();
        for _ in 0..50 {
            text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.contains("message_end:") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        text
    };
    assert!(
        body.contains("message_end:assistant"),
        "扩展应收到带 role 的 message_end（改前一次都没有）。日志：\n{body}"
    );
}
