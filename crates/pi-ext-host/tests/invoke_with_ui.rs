//! E4f / S2：工具调用期间 `custom_input` 必须能即时送达扩展，且 invoke 毫秒级返回。
//!
//! 修前：`Cmd::Invoke` 用 `runtime.block_on` 独占 actor，按键要等 invoke 超时才被处理
//! （实测 `invoke` 在 3s/20s 超时处才返回，见 `notes/p9-1e4e/status.md`）。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pi_ext_host::{HostHandle, HostOptions, UiFrame};
use serde_json::json;

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
fn custom_input_is_delivered_while_invoke_is_in_flight() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/custom_invoke-ext.mjs")],
        cwd: repo_root(),
        env: Vec::new(),
    })
    .expect("宿主应能启动");
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_ui_sink(Some(tx));
    host.set_ctx(
        json!({"cwd": repo_root().display().to_string(), "mode": "tui", "hasUI": true,
        "model": "mock-model", "thinkingLevel": "off"}),
    );

    let started = Instant::now();
    let invoke = {
        let host = host.clone();
        std::thread::spawn(move || {
            host.invoke("ext_ask_inline", json!({}), Duration::from_secs(30))
        })
    };
    let mut opened = false;
    for _ in 0..50 {
        if let Ok(UiFrame::CustomOpen { .. }) = rx.recv_timeout(Duration::from_millis(100)) {
            opened = true;
            break;
        }
    }
    assert!(opened, "工具内应打开自定义面板");
    let sent = Instant::now();
    let done = host.custom_input("\r", 80).expect("按键应成功");
    assert_eq!(done["closed"], true, "回车应关闭面板：{done}");
    let result = invoke.join().expect("线程应结束").expect("工具应成功");
    let elapsed = started.elapsed();
    eprintln!(
        "MEASURE: custom_input 往返 {:?}，invoke 总耗时 {:?}",
        sent.elapsed(),
        elapsed
    );
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("answer:yes"),
        "工具应拿到组件返回值：{result}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "invoke 应在按键后立即返回（实测 {elapsed:?}）"
    );
    host.shutdown();
}
