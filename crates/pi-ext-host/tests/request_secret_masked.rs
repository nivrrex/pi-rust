//! E4f / S5：真实 `request_secret`（pi-secrets）的掩码输入：明文不落盘、只报长度。
//!
//! 隔离：`HOME` 与 `PI_CODING_AGENT_DIR` 都指向临时目录（本批禁止写 `~/.pi`）。
//! 由 `PI_MATRIX_EXTENSIONS_DIR` 门控（值为含 `pi-secrets` 的 /tmp 副本根）。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{HostHandle, HostOptions, UiFrame};
use serde_json::json;

const PLAINTEXT: &str = "s3cr3t-value-xyz";
const NAME: &str = "RS_TEST_SECRET";

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
fn request_secret_masks_and_does_not_leak() {
    let Ok(root) = std::env::var("PI_MATRIX_EXTENSIONS_DIR") else {
        eprintln!("跳过：未设置 PI_MATRIX_EXTENSIONS_DIR");
        return;
    };
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent_dir = tmp.path().join("agent");
    std::fs::create_dir_all(agent_dir.join("extensions")).expect("建目录");
    copy_dir(
        &PathBuf::from(&root).join("agent/extensions/pi-secrets"),
        &agent_dir.join("extensions/pi-secrets"),
    );
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("建 home");
    let extensions = pi_ext_host::discover_extensions_in_order(tmp.path(), &agent_dir);
    assert!(!extensions.is_empty(), "副本里应有 pi-secrets");
    let host = HostHandle::start(HostOptions {
        node,
        runner: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/ext-host/runner.mjs"),
        extensions,
        cwd: tmp.path().to_path_buf(),
        env: vec![
            (
                "PI_CODING_AGENT_DIR".to_string(),
                agent_dir.display().to_string(),
            ),
            ("HOME".to_string(), home.display().to_string()),
        ],
    })
    .expect("宿主应能启动");
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_ui_sink(Some(tx));
    host.set_ctx(
        json!({"cwd": tmp.path().display().to_string(), "mode": "tui", "hasUI": true,
        "model": "mock-model", "thinkingLevel": "off"}),
    );

    let invoke = {
        let host = host.clone();
        std::thread::spawn(move || {
            host.invoke(
                "request_secret",
                json!({"name": NAME, "reason": "test"}),
                Duration::from_secs(20),
            )
        })
    };
    let mut panel = None;
    for _ in 0..60 {
        if let Ok(UiFrame::CustomOpen { lines, .. }) = rx.recv_timeout(Duration::from_millis(100)) {
            panel = Some(lines);
            break;
        }
    }
    let panel = panel.expect("应打开掩码输入面板");
    for ch in PLAINTEXT.chars() {
        let _ = host.custom_input(&ch.to_string(), 100);
    }
    let rendered = host
        .custom_input("", 100)
        .map(|v| v.to_string())
        .unwrap_or_default();
    assert!(
        !rendered.contains(PLAINTEXT) && !panel.concat().contains(PLAINTEXT),
        "掩码面板不应回显明文"
    );
    let _ = host.custom_input("\r", 100);
    let result = invoke.join().expect("线程").expect("工具应成功");
    let text = result
        .get("content")
        .map(|c| c.to_string())
        .unwrap_or_default();
    assert!(!text.contains(PLAINTEXT), "工具结果不应包含明文：{text}");
    assert!(text.contains(NAME), "应报告名字：{text}");
    let mut hits = 0;
    for path in walk(tmp.path()) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if content.contains(PLAINTEXT) {
                hits += 1;
            }
        }
    }
    assert_eq!(hits, 0, "明文不应落盘（命中文件数 {hits}）");
    host.shutdown();
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("建目录");
    for entry in std::fs::read_dir(from).expect("读目录") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            let _ = std::fs::copy(entry.path(), target);
        }
    }
}

fn walk(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}
