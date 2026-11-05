//! P9-F6/S2：真实 `ask_user`（pi-interact）的**多选**与**取消**两条路径。
//!
//! 单选路径见 `ask_user_custom.rs`。
//!
//! 由 `PI_MATRIX_EXTENSIONS_DIR` 指定 /tmp 副本，未设置时跳过。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{HostHandle, HostOptions, UiFrame};
use serde_json::json;

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn harness() -> Option<(HostHandle, std::sync::mpsc::Receiver<UiFrame>)> {
    let root = std::env::var("PI_MATRIX_EXTENSIONS_DIR").ok()?;
    let node = node_path()?;
    let agent_dir = PathBuf::from(&root).join("agent");
    let cwd = PathBuf::from(&root).join("work");
    std::fs::create_dir_all(&cwd).ok()?;
    let extensions = pi_ext_host::discover_extensions_in_order(&cwd, &agent_dir);
    if extensions.is_empty() {
        return None;
    }
    let host = HostHandle::start(HostOptions {
        node,
        runner: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/ext-host/runner.mjs"),
        extensions,
        cwd: cwd.clone(),
        env: vec![(
            "PI_CODING_AGENT_DIR".to_string(),
            agent_dir.display().to_string(),
        )],
    })
    .ok()?;
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_ui_sink(Some(tx));
    host.set_ctx(
        json!({"cwd": cwd.display().to_string(), "mode": "tui", "hasUI": true,
        "model": "mock-model", "thinkingLevel": "off"}),
    );

    Some((host, rx))
}

fn wait_open(rx: &std::sync::mpsc::Receiver<UiFrame>) -> Option<Vec<String>> {
    for _ in 0..100 {
        if let Ok(UiFrame::CustomOpen { lines, .. }) = rx.recv_timeout(Duration::from_millis(100)) {
            return Some(lines);
        }
    }
    None
}

#[test]
fn ask_user_multiselect_toggles_two_options() {
    let Some((host, rx)) = harness() else {
        eprintln!("跳过：未设置 PI_MATRIX_EXTENSIONS_DIR 或本机没有 node");
        return;
    };
    let call = {
        let host = host.clone();
        std::thread::spawn(move || {
            host.invoke(
                "ask_user",
                json!({"question": "pick many", "type": "select", "multiple": true,
                       "options": [{"label": "alpha", "value": "a"},
                                   {"label": "beta", "value": "b"},
                                   {"label": "gamma", "value": "c"}]}),
                Duration::from_secs(10),
            )
        })
    };
    assert!(wait_open(&rx).is_some(), "应打开多选面板");
    let _ = host.custom_input(" ", 100).expect("勾选第一项");
    let _ = host.custom_input("\u{1b}[B", 100).expect("下移");
    let _ = host.custom_input(" ", 100).expect("勾选第二项");
    let done = host.custom_input("\r", 100).expect("提交");
    assert_eq!(done["closed"], true, "提交后应关闭：{done}");
    let result = call.join().expect("线程").expect("工具应成功");
    let text = result
        .get("content")
        .map(|c| c.to_string())
        .unwrap_or_default();
    assert!(text.contains("answered"), "多选应返回 answered：{text}");
    host.shutdown();
}

#[test]
fn ask_user_escape_cancels() {
    let Some((host, rx)) = harness() else {
        eprintln!("跳过：未设置 PI_MATRIX_EXTENSIONS_DIR 或本机没有 node");
        return;
    };
    let call = {
        let host = host.clone();
        std::thread::spawn(move || {
            host.invoke(
                "ask_user",
                json!({"question": "pick one", "type": "select", "multiple": false,
                       "options": [{"label": "alpha", "value": "a"}]}),
                Duration::from_secs(10),
            )
        })
    };
    assert!(wait_open(&rx).is_some(), "应打开面板");
    let done = host.custom_input("\u{1b}", 100).expect("Esc");
    assert_eq!(done["closed"], true, "Esc 后应关闭：{done}");
    let result = call.join().expect("线程").expect("工具应成功");
    let text = result
        .get("content")
        .map(|c| c.to_string())
        .unwrap_or_default();
    assert!(
        text.contains("cancelled"),
        "Esc 应返回 cancelled（index.ts:330）：{text}"
    );
    host.shutdown();
}
