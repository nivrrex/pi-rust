//! F61/28：`ctx.ui.setToolsExpanded(bool)` 必须**真的改变 TUI 的展开态**。
//!
//! 改前：宿主只把这个值记进自己的状态并回执，TUI 完全不知道
//! （`crates/pi-ext-host/src/client.rs` 的注释原文："这里只记录状态并回执"）。
//! 已装扩展 `pi-remote-control` 用它在远程端切换展开态（`adapter.ts:463`），
//! 于是**远程切换没有任何效果**——这正是 LIMITATIONS F28 那条限制的推翻条件。
//!
//! 判据用状态行 `Tool output: expanded`（与 Ctrl+O 同一句文案，
//! ref `interactive-mode.ts:4419`）：它是"渲染层真的接到了新状态"的直接证据。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn screen_of(raw: &std::path::Path) -> String {
    let bytes = std::fs::read(raw).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let mut out = String::new();
    let mut rest = text.as_ref();
    while let Some(index) = rest.find('\u{1b}') {
        out.push_str(&rest[..index]);
        let after = &rest[index..];
        let mut chars = after.chars();
        let _ = chars.next();
        match chars.next() {
            Some('[') => {
                let mut consumed = 2;
                for ch in after.chars().skip(2) {
                    consumed += ch.len_utf8();
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                }
                rest = &after[consumed.min(after.len())..];
            }
            Some(']') => {
                if let Some(bell) = after.find('\u{7}') {
                    rest = &after[bell + 1..];
                } else {
                    rest = "";
                }
            }
            _other => {
                rest = &after[1.min(after.len())..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 扩展在 `session_start` 里要求展开工具输出，屏幕应显示 `Tool output: expanded`。
#[test]
fn extension_can_expand_tool_output() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    for dir in [&agent, &home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m1"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m1","name":"m1","contextWindow":200000,"maxTokens":1024}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");
    // 探针扩展：只做一件事——要求展开工具输出。
    let ext = agent.join("extensions/expander");
    std::fs::create_dir_all(&ext).expect("建目录");
    std::fs::write(
        ext.join("index.mjs"),
        r#"export default function expander(pi) {
  pi.on("session_start", async (_event, ctx) => {
    ctx.ui.setToolsExpanded(true);
  });
}
"#,
    )
    .expect("写扩展");

    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 3000, "timeoutMs": 90000}},
        {"mark": "after-start"},
        {"text": "\u{4}"},
        {"delayMs": 1500}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(&raw)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("COLORTERM", "")
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("运行 pty-helper");
    assert!(
        output.status.success(),
        "pty-helper 应成功（stderr：{}）",
        String::from_utf8_lossy(&output.stderr)
    );
    let screen = screen_of(&raw);
    assert!(
        screen.contains("Tool output: expanded"),
        "扩展调用 setToolsExpanded(true) 后，TUI 应真的展开并显示该状态行。屏幕：\n{screen}"
    );
}
