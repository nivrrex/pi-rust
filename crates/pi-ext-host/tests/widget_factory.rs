//! 组件工厂式 `ctx.ui.setWidget`：只接受 `render() -> string[]` 的组件（pi-remote-control 的 QR 形态）。

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
fn widget_factory_render_lines_reach_the_host() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/widgetfactory-ext.mjs")],
        cwd: repo_root(),
        env: Vec::new(),
    })
    .expect("宿主应能启动");
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_ui_sink(Some(tx));
    host.set_ctx(serde_json::json!({
        "cwd": repo_root().display().to_string(),
        "mode": "tui",
        "hasUI": true,
        "model": "mock-model",
        "thinkingLevel": "off"
    }));
    host.emit(
        "session_start",
        serde_json::json!({"type": "session_start", "reason": "startup"}),
        Duration::from_secs(10),
    )
    .expect("session_start 应成功");
    let mut lines: Vec<String> = Vec::new();
    while let Ok(frame) = rx.recv_timeout(Duration::from_millis(200)) {
        if let UiFrame::SetWidget {
            key,
            lines: Some(widget_lines),
            placement,
            ..
        } = frame
        {
            assert_eq!(key, "factory");
            assert_eq!(placement, "belowEditor");
            lines = widget_lines;
        }
    }
    // `theme.fg("accent", …)` 现在会带上 ref 的 truecolor SGR（色表由 dark.json 生成），
    // 比较时剥掉 ANSI，只校验文本。
    let plain: Vec<String> = lines
        .iter()
        .map(|line| {
            let mut out = String::new();
            let mut chars = line.chars();
            while let Some(ch) = chars.next() {
                if ch == '\u{1b}' {
                    for next in chars.by_ref() {
                        if next == 'm' {
                            break;
                        }
                    }
                    continue;
                }
                out.push(ch);
            }
            out
        })
        .collect();
    assert_eq!(plain, vec!["factory-line-1", "factory-line-2"]);
    host.shutdown();
}
