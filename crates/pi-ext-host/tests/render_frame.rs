//! 渲染钩子的端到端：扩展返回 `Text`/`Spacer`/`Container` 树 ⇒ 序列化成帧；
//! 用了不支持的组件 ⇒ `unsupported`（宿主回落内置渲染）。

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
fn render_hooks_serialize_to_frames() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/render-ext.mjs")],
        cwd: repo_root(),
        env: Vec::new(),
    })
    .expect("宿主应能启动");

    let frame = host
        .render(
            "ext_render_ok",
            "call",
            serde_json::json!({"args": {"x": 1}}),
            80,
        )
        .expect("渲染应成功")
        .expect("应有帧");
    assert_eq!(frame["kind"], "stack");
    assert_eq!(frame["children"][0]["kind"], "text");
    assert!(frame["children"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("call:"));
    assert_eq!(frame["children"][1]["kind"], "spacer");

    let unsupported = host
        .render(
            "ext_render_unsupported",
            "call",
            serde_json::json!({"args": {}}),
            80,
        )
        .expect("渲染应成功")
        .expect("应有帧");
    assert_eq!(unsupported["kind"], "unsupported");
    assert!(unsupported["reason"]
        .as_str()
        .unwrap_or_default()
        .contains("SelectList"));

    // 没有渲染器的工具 ⇒ 没有帧。
    let none = host
        .render("ext_echo", "call", serde_json::json!({"args": {}}), 80)
        .expect("调用应成功");
    assert!(none.is_none(), "没有渲染器时应返回 None");
    host.shutdown();
    std::thread::sleep(Duration::from_millis(20));
}
