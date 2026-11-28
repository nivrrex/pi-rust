//! F60/8：`context` 事件必须派发，且扩展的改写必须被采用。
//!
//! ref `runner.ts:1190-1219` 的 `emitContext`：请求前把**非 system** 消息交给扩展，
//! 扩展可返回新列表或原地改写，宿主再把 system 消息还原回原位。
//!
//! 真实使用方 `context-gateway`（用户已装）在这里做**投影层老化**
//! （`index.ts:320-`：重复发送过的工具结果全文换成视图，会话原史不动）。
//! 此前 pi-rs 完全不派发该事件 ⇒ 老化层从未生效，上下文只增不减。
//!
//! 本用例覆盖三条语义：
//! 1. 扩展**看不到** system 消息；
//! 2. 扩展返回的 messages **被宿主采用**（改写真的进请求）；
//! 3. system 消息**被还原**回原位（不影响提示词）。

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
    let target = agent.join("extensions/context-probe");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/context-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

fn user(text: &str) -> pi_ai::message::Message {
    pi_ai::message::Message::User {
        content: pi_ai::UserContent::Text(text.to_string()),
        timestamp: 0,
        extra: serde_json::Map::new(),
    }
}

/// 一次完整的 `transform_context` 往返：断言 system 不可见、返回值被采用、system 被还原。
#[test]
fn context_hook_rewrites_messages_and_keeps_system() {
    let Some(_node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install(agent.path());
    let log = agent.path().join("probe.log");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("node"));
    std::env::set_var("PI_EXT_TEST_LOG", &log);
    pi_cli::ext_tools::configure_ctx(agent.path(), "tui", true);
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000c8".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();

    // system 的真实形态是 `content: ""` + `sections`/`toolsAdded` 放在 `extra` 里
    // （serde flatten）。用同一个形状，确保"还原 system"测的是生产路径的形态。
    let mut system_extra = serde_json::Map::new();
    system_extra.insert(
        "sections".to_string(),
        serde_json::json!({ "preamble": "you are a test" }),
    );
    let system = pi_ai::message::Message::System {
        content: pi_ai::UserContent::Text(String::new()),
        timestamp: 0,
        extra: system_extra,
    };
    let input = vec![system.clone(), user("第一问"), user("第二问")];
    let output = pi_cli::ext_tools::transform_context(input);

    let body = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        body.contains("context:count="),
        "扩展应收到 context 事件（改前一次都没有）。日志：\n{body}"
    );
    assert!(
        body.contains("context:has_system=false"),
        "context 事件不应把 system 消息交给扩展。日志：\n{body}"
    );

    // 扩展在第一条 user 前加了 `[aged] `，宿主应采用返回值。
    // user 的 content 回来时可能是块数组形态（ref 的 `AgentMessage` 就是数组），
    // 两种形态都要能取到文本。
    let texts: Vec<String> = output
        .iter()
        .filter_map(|message| match message {
            pi_ai::message::Message::User { content, .. } => Some(match content {
                pi_ai::UserContent::Text(text) => text.clone(),
                pi_ai::UserContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        pi_ai::message::Content::Text { text, .. } => Some(text.clone()),
                        _other => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            }),
            _other => None,
        })
        .collect();
    assert!(
        texts
            .first()
            .is_some_and(|text| text.starts_with("[aged] ")),
        "扩展返回的 messages 应被采用（第一条 user 带 [aged] 前缀），实际：{texts:?}"
    );
    // system 必须还原（在 index 0）。
    assert!(
        matches!(output.first(), Some(pi_ai::message::Message::System { .. })),
        "system 消息应被还原到开头，实际首条：{:?}",
        output.first()
    );
    assert_eq!(output.len(), 3, "消息条数应保持不变");
}
