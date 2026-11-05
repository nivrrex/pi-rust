//! 阻塞式 UI 询问的宿主侧往返：`host.ui.select/input/confirm` 由前端回答后回填。
//!
//! 本测试用一个"假前端"线程应答（TUI 面板在 pi-cli 里接，属于下一批）。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{HostHandle, HostOptions, UiAsk};

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
fn ui_asks_roundtrip_through_the_frontend() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let log = tmp.path().join("events.log");
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/ui-select-ext.mjs")],
        cwd: repo_root(),
        env: vec![("PI_EXT_TEST_LOG".to_string(), log.display().to_string())],
    })
    .expect("宿主应能启动");
    // 扩展只有在 `ctx.hasUI` 为真时才会走面板（ref `runner.ts:320-325` 的 noOpUIContext）。
    host.set_ctx(serde_json::json!({
        "cwd": repo_root().display().to_string(),
        "mode": "tui",
        "hasUI": true,
        "model": null,
        "thinkingLevel": null
    }));
    // 假前端：select → "green"，input → "tester"，confirm → true。
    let (tx, rx) = std::sync::mpsc::channel::<UiAsk>();
    host.set_ui_ask(Some(tx));
    let frontend = std::thread::spawn(move || {
        for ask in rx {
            let answer = match ask.kind.as_str() {
                "select" => serde_json::json!("green"),
                "input" => serde_json::json!("tester"),
                "confirm" => serde_json::json!(true),
                _other => serde_json::Value::Null,
            };
            let _ = ask.reply.send(answer);
        }
    });
    host.emit(
        "session_start",
        serde_json::json!({"type": "session_start", "reason": "startup"}),
        Duration::from_secs(20),
    )
    .expect("session_start 应成功");
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.contains("selected:green:tester:true"),
        "三个询问都应由前端回答：{text}"
    );
    host.shutdown();
    drop(host);
    let _ = frontend.join();
}
