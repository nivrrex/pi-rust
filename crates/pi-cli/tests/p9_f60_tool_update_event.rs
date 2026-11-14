//! F60/9：`tool_execution_update` 也必须派发给扩展（ref `agent-session.ts:1104-1110`）。
//!
//! `pi-ai-summary`（用户已装）订阅它记录**工具执行中**的诊断快照
//! （`src/events.ts:123-135`；`partialResult` 是累计快照而非增量）。
//! 此前 pi-rs 不派发 ⇒ 该扩展看到的一直是 `undefined`。
//!
//! 本用例从**宿主侧**验证派发链路：宿主的 `emit` 把 `tool_execution_update`
//! 原样交给扩展，载荷字段名与 ref 一致（`toolCallId` / `toolName` /
//! `args` / `partialResult`）。

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
    let target = agent.join("extensions/tool-update-probe");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body = r#"
// F60/9 探针：把 tool_execution_update 的载荷字段写进日志。
import fs from "node:fs";
function record(line) {
  const path = process.env.PI_EXT_TEST_LOG;
  if (!path) return;
  try { fs.appendFileSync(path, `${line}\n`); } catch { /* ignore */ }
}
export default function toolUpdateProbe(pi) {
  pi.on("tool_execution_update", (event) => {
    record(
      `tool_execution_update:id=${event?.toolCallId}:name=${event?.toolName}:` +
      `has_partial=${event?.partialResult !== undefined}`,
    );
  });
}
"#;
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 宿主派发的 `tool_execution_update` 必须带齐 ref 的四个字段。
#[test]
fn tool_execution_update_reaches_extensions_with_ref_fields() {
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
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000c9".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();

    pi_cli::ext_tools::emit_event(
        "tool_execution_update",
        serde_json::json!({
            "type": "tool_execution_update",
            "toolCallId": "call-1",
            "toolName": "bash",
            "args": { "command": "echo hi" },
            "partialResult": { "content": [{ "type": "text", "text": "hi" }] },
        }),
    );

    let body = {
        let mut text = String::new();
        for _ in 0..50 {
            text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.contains("tool_execution_update:") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        text
    };
    assert!(
        body.contains("tool_execution_update:id=call-1:name=bash:has_partial=true"),
        "扩展应收到带 toolCallId/toolName/partialResult 的 tool_execution_update。日志：\n{body}"
    );
}
