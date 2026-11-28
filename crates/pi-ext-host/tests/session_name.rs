//! `pi.getSessionName()`（ref `types.ts:1509`）：**未设名字时**返回 `undefined`。
//!
//! 会话命名本身已支持（`pi.setSessionName` 写入后 `ctx.sessionName` 即时刷新，
//! 见 `crates/pi-cli/src/ext_tools.rs` 的 `append_session_info*` 出口）。
//! 本用例只覆盖"没设过名字"这一分支。

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
fn session_name_is_undefined_before_a_name_is_set() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let log = tmp.path().join("events.log");
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/sessionname-ext.mjs")],
        cwd: repo_root(),
        env: vec![("PI_EXT_TEST_LOG".to_string(), log.display().to_string())],
        log_path: None,
        log_sink: None,
    })
    .expect("宿主应能启动");
    host.emit(
        "session_start",
        serde_json::json!({"type": "session_start", "reason": "startup"}),
        Duration::from_secs(10),
    )
    .expect("session_start 应成功");
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(text.contains("name:undefined"), "应为 undefined：{text}");
    host.shutdown();
}
