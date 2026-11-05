//! `theme.fg` 的色表与未知色名（ref `theme.ts:323-327`：未知色名**抛错**，不静默回落）。

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
fn unknown_theme_color_falls_back_to_unsupported() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/render-unknown-color-ext.mjs")],
        cwd: repo_root(),
        env: Vec::new(),
    })
    .expect("宿主应能启动");
    let frame = host
        .render(
            "ext_render_bad_color",
            "call",
            serde_json::json!({"args": {}}),
            80,
        )
        .expect("调用应成功")
        .expect("应有帧");
    assert_eq!(frame["kind"], "unsupported");
    assert!(
        frame["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("Unknown theme color"),
        "应回传 ref 的报错文案：{}",
        frame["reason"]
    );
    host.shutdown();
    std::thread::sleep(Duration::from_millis(20));
}
