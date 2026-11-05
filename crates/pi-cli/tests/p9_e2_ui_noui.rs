//! P9 E2：无 UI（print/rpc）下 `ctx.ui.select/input/confirm` 的语义。
//!
//! ref `runner.ts:320-325` 的 `noOpUIContext`：`select`/`input` → `undefined`、
//! `confirm` → `false`、`notify`/`setStatus` 空操作（hasUI=false 时用它）。

use std::path::{Path, PathBuf};
use std::process::Command;

fn node_path() -> Option<PathBuf> {
    let output = Command::new("sh")
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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn prepare(agent: &Path) {
    let target = agent.join("extensions/noui");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/ui-noui-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

#[test]
fn no_ui_context_returns_cancel_values() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("cfg");
    std::fs::create_dir_all(&agent).expect("建目录");
    prepare(&agent);
    let log = tmp.path().join("events.log");
    std::env::set_var("PI_CODING_AGENT_DIR", &agent);
    std::env::set_var("PI_NODE", &node);
    std::env::set_var("PI_EXT_TEST_LOG", &log);
    pi_cli::ext_tools::configure_ctx(&agent, "print", false);
    // F28/S1：`session_start` 推迟到会话登记之后派发；这里登记 id 以触发启动时的派发。
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-000000000000".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();

    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.contains("noui:undefined:undefined:false:hasUI=false"),
        "无 UI 时应返回取消语义：{text}"
    );
}
