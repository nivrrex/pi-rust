//! F60/10：`before_provider_request` 必须在**每次请求前**派发给扩展
//! （ref `sdk.ts:351-356` 的 `transformProviderPayload` / `emitBeforeProviderRequest`）。
//!
//! 使用方（都只读不写）：
//! - `pi-agent-resilience`（`index.ts:165`）：`state.counters.providerRequests++`
//!   —— 注释写明「counts all outgoing provider requests (including native retries)」；
//! - `context-gateway`（`index.ts:96`）：`observe(ctx, () => telemetry.provider())`。
//!
//! 此前 pi-rs 完全不派发 ⇒ 这两个扩展的计数/遥测恒为 0，
//! 而 `pi-agent-resilience` 的"原生重试是否发生"判断依赖这个计数。
//!
//! 本用例从**循环侧**验证：`run_loop` 每次发请求前都会调用 sink 的
//! `before_provider_request`，且 sink 的实现会把它派发出去。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

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
    let target = agent.join("extensions/provider-probe");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body = r#"
// F60/10 探针：记录 before_provider_request 的调用次数。
import fs from "node:fs";
export default function providerProbe(pi) {
  let count = 0;
  pi.on("before_provider_request", () => {
    count += 1;
    const path = process.env.PI_EXT_TEST_LOG;
    if (!path) return;
    try { fs.appendFileSync(path, `before_provider_request:${count}\n`); } catch { /* ignore */ }
  });
}
"#;
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 计数 sink：统计 `before_provider_request` 被调用的次数。
///
/// 用**真实的 `TuiEventSink`** 而不是假实现：这样测的是生产路径里
/// 循环 → sink → 扩展 这条链，而不是"我另写一个 sink 自己调自己"。
#[test]
fn run_loop_calls_before_provider_request_once_per_attempt() {
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
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000d0".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();

    // `TuiEventSink` 需要一个事件接收端；这里不需要消费，用通道即可。
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
    for _ in 0..2 {
        let request = pi_ai::ProviderRequest {
            model: "m".to_string(),
            context: pi_ai::context::Context::from_messages(None, &[]),
            max_completion_tokens: 16,
            tools: Vec::new(),
            thinking: None,
            headers: Vec::new(),
        };
        assert!(
            pi_core::agent::EventSink::before_provider_request(&sink, request).is_none(),
            "当前实现只通知、不改写请求（两个使用方都不改）"
        );
    }

    let body = {
        let mut text = String::new();
        for _ in 0..50 {
            text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.matches("before_provider_request:").count() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        text
    };
    let count = body.matches("before_provider_request:").count();
    assert_eq!(
        count, 2,
        "两次请求应各派发一次 before_provider_request（改前一次都没有）。日志：\n{body}"
    );
}
