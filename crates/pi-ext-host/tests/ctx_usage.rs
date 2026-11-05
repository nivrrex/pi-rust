//! `ctx.getContextUsage()`（ref `types.ts:294-300`）：宿主把用量放进 ctx 快照，扩展读到同一份。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{HostHandle, HostOptions};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

#[test]
fn context_usage_reaches_the_extension() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let log = tmp.path().join("events.log");
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/usage-ext.mjs")],
        cwd: repo_root(),
        env: vec![("PI_EXT_TEST_LOG".to_string(), log.display().to_string())],
    })
    .expect("宿主应能启动");
    host.set_ctx(serde_json::json!({
        "cwd": repo_root().display().to_string(),
        "mode": "tui",
        "hasUI": true,
        "model": "mock-model",
        "thinkingLevel": "off",
        "usage": {"tokens": 1234, "contextWindow": 128000, "percent": 0.96}
    }));
    host.emit(
        "agent_end",
        serde_json::json!({"type": "agent_end", "messages": []}),
        Duration::from_secs(10),
    )
    .expect("agent_end 应成功");
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.contains("usage:1234:128000:0.96"),
        "扩展应读到宿主下发的用量：{text}"
    );
    host.shutdown();
}
