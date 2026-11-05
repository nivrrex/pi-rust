//! `ctx.ui.setFooter`（ref `types.ts:189-193`）：工厂只接受 `render() -> string[]`。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{HostHandle, HostOptions, UiFrame};

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
fn set_footer_lines_reach_the_host() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/footer-ext.mjs")],
        cwd: repo_root(),
        env: Vec::new(),
    })
    .expect("宿主应能启动");
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_ui_sink(Some(tx));
    host.set_ctx(serde_json::json!({
        "cwd": repo_root().display().to_string(),
        "mode": "tui", "hasUI": true, "model": "mock-model", "thinkingLevel": "off"
    }));
    host.emit(
        "session_start",
        serde_json::json!({"type": "session_start", "reason": "startup"}),
        Duration::from_secs(10),
    )
    .expect("session_start 应成功");
    let mut found = None;
    while let Ok(frame) = rx.recv_timeout(Duration::from_millis(200)) {
        if let UiFrame::SetFooter { lines } = frame {
            found = lines;
        }
    }
    let lines = found.expect("应收到 SetFooter");
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("custom-footer providers=1"));
    assert_eq!(lines[1], "second-footer-line");
    host.shutdown();
}
