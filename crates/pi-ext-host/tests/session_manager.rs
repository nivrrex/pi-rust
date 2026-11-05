//! `ctx.sessionManager` 的只读子集（ref `types.ts:1854`）：
//! 宿主给会话文件路径，扩展按需读文件解析条目。

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
fn session_manager_exposes_the_file_and_entries() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let session = tmp.path().join("session.jsonl");
    std::fs::write(&session, "{\"type\":\"session\"}\n{\"type\":\"message\"}\n")
        .expect("写会话文件");
    let log = tmp.path().join("events.log");
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/sessionmgr-ext.mjs")],
        cwd: repo_root(),
        env: vec![("PI_EXT_TEST_LOG".to_string(), log.display().to_string())],
    })
    .expect("宿主应能启动");
    host.set_ctx(serde_json::json!({
        "cwd": repo_root().display().to_string(),
        "mode": "print",
        "hasUI": false,
        "model": "mock-model",
        "thinkingLevel": "off",
        "sessionFile": session.display().to_string(),
    }));
    host.emit(
        "agent_end",
        serde_json::json!({"type": "agent_end", "messages": []}),
        Duration::from_secs(10),
    )
    .expect("agent_end 应成功");
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.contains("sm:2:file"),
        "扩展应读到 2 条条目与文件路径：{text}"
    );
    host.shutdown();
}
