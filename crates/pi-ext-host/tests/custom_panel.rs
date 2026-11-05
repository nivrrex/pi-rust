//! `ctx.ui.custom` 的最小交互（ref `types.ts:201-215`）：开面板 → 按键下行 → done 回包。

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
fn custom_panel_opens_accepts_keys_and_returns_done() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let log = tmp.path().join("events.log");
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/custom-ext.mjs")],
        cwd: repo_root(),
        env: vec![("PI_EXT_TEST_LOG".to_string(), log.display().to_string())],
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

    // 开面板：初始行里第一项被选中。
    let mut opened = None;
    while let Ok(frame) = rx.recv_timeout(Duration::from_millis(200)) {
        if let UiFrame::CustomOpen { lines, .. } = frame {
            opened = Some(lines);
        }
    }
    let lines = opened.expect("应收到 CustomOpen");
    assert_eq!(lines, vec!["→ alpha".to_string(), "  beta".to_string()]);

    // 下移 + 回车：done 返回 "beta"。
    let moved = host.custom_input("\u{1b}[B", 80).expect("按键应成功");
    assert_eq!(moved["closed"], false);
    assert_eq!(moved["lines"][0], "  alpha");
    assert_eq!(moved["lines"][1], "→ beta");

    let done = host.custom_input("\r", 80).expect("回车应成功");
    assert_eq!(done["closed"], true);
    assert_eq!(done["result"], "beta");

    // `custom()` 的 Promise 在扩展侧异步 resolve，日志可能晚一拍。
    let mut text = String::new();
    for _ in 0..40 {
        text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.contains("custom:result:beta") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        text.contains("custom:result:beta"),
        "扩展应拿到结果：{text}"
    );
    host.shutdown();
}
