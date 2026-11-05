//! 真实 `ask_user`（pi-interact）走 `ctx.ui.custom`：开面板 → 勾选 → 确认。
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

#[test]
fn ask_user_runs_through_custom_panel() {
    let Ok(root) = std::env::var("PI_MATRIX_EXTENSIONS_DIR") else {
        eprintln!("跳过：未设置 PI_MATRIX_EXTENSIONS_DIR");
        return;
    };
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent_dir = PathBuf::from(&root).join("agent");
    let cwd = PathBuf::from(&root).join("work");
    std::fs::create_dir_all(&cwd).expect("建目录");
    let extensions = pi_ext_host::discover_extensions_in_order(&cwd, &agent_dir);
    assert!(!extensions.is_empty(), "副本里应能发现扩展");
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
    .expect("宿主应能启动");
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_ui_sink(Some(tx));
    host.set_ctx(
        json!({"cwd": cwd.display().to_string(), "mode": "tui", "hasUI": true,
        "model": "mock-model", "thinkingLevel": "off"}),
    );

    let invoke = {
        let host = host.clone();
        std::thread::spawn(move || {
            host.invoke(
                "ask_user",
                json!({"question": "pick one", "type": "select", "multiple": false,
                       "options": [{"label": "alpha", "value": "a"},
                                   {"label": "beta", "value": "b"}]}),
                Duration::from_secs(10),
            )
        })
    };
    let mut opened = None;
    for _ in 0..100 {
        if let Ok(UiFrame::CustomOpen { lines, .. }) = rx.recv_timeout(Duration::from_millis(100)) {
            opened = Some(lines);
            break;
        }
    }
    let lines = opened.expect("ask_user 应打开自定义面板");
    assert!(!lines.is_empty(), "面板应有内容：{lines:?}");

    // 空格勾选第一项，回车确认。
    let _ = host.custom_input(" ", 100).expect("空格");
    let done = host.custom_input("\r", 100).expect("回车");
    assert_eq!(done["closed"], true, "回车后应关闭面板：{done}");
    let result = invoke.join().expect("线程应结束").expect("工具应成功");
    let text = result
        .get("content")
        .map(|c| c.to_string())
        .unwrap_or_default();
    assert!(
        text.contains("\"selected\":\"a\"") || text.contains("answered"),
        "工具应返回所选结果：{text}"
    );
    host.shutdown();
}
