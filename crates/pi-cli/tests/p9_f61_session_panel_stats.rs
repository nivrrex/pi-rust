//! F61/7：`/session` 面板必须显示**真实统计**（ref `interactive-mode.ts:6432-6470`）。
//!
//! 此前面板里的 `Total/User/Assistant/Tools/Input/Output/Total` 全是**硬编码 0**：
//! 用户打开 `/session` 只会看到"会话是空的"，与会话真实状态无关
//! （真实会话里有几十条消息、几十万 token，面板依然全 0）。
//!
//! 本用例用一份含 1 条 user、1 条 assistant（带 usage）、1 条 toolResult 的会话，
//! 断言面板里的数字与这些条目一致。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::path::{Path, PathBuf};
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

fn screen_of(raw: &Path) -> String {
    let bytes = std::fs::read(raw).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if next == '\u{7}' {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[test]
fn session_panel_reports_real_counts() {
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
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m","name":"m","contextWindow":200000,"maxTokens":1024}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");

    // 会话：1 user + 1 assistant（带 1 个 toolCall、usage）+ 1 toolResult。
    let session = sessions.join("2026-01-01T00-00-00-000Z_sp.jsonl");
    let header = serde_json::json!({
        "type": "session", "id": "s0", "parentId": serde_json::Value::Null,
        "timestamp": "2026-01-01T00:00:00Z", "cwd": work.display().to_string(), "version": 3,
    });
    let user = serde_json::json!({
        "type": "message", "id": "u1", "parentId": "s0", "timestamp": "2026-01-01T00:00:01Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "读文件"}]},
    });
    let assistant = serde_json::json!({
        "type": "message", "id": "a1", "parentId": "u1", "timestamp": "2026-01-01T00:00:02Z",
        "message": {
            "role": "assistant",
            "content": [{"type": "text", "text": "我来读"}, {"type": "toolCall", "id": "tc1",
                "name": "read", "arguments": {"path": "a.txt"}}],
            "api": "openai-completions", "provider": "p", "model": "m",
            "usage": {"input": 1000, "output": 50, "cacheRead": 200, "cacheWrite": 0,
                "totalTokens": 1250, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": "toolUse", "timestamp": 0,
        },
    });
    let tool_result = serde_json::json!({
        "type": "message", "id": "r1", "parentId": "a1", "timestamp": "2026-01-01T00:00:03Z",
        "message": {"role": "toolResult", "toolCallId": "tc1", "toolName": "read",
            "content": [{"type": "text", "text": "文件内容"}], "isError": false, "timestamp": 3},
    });
    let mut file = std::fs::File::create(&session).expect("建会话");
    for entry in [header, user, assistant, tool_result] {
        writeln!(file, "{entry}").expect("写会话");
    }
    drop(file);

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"/session\r"},{"delayMs":3000},{"mark":"panel"},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    )
    .expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("34")
        .arg("--exit-code-file")
        .arg(&code)
        .arg("--out")
        .arg(&raw)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--session")
        .arg(&session)
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
    // 3 条 message 条目：1 user、1 assistant、1 toolResult；1 次工具调用。
    // `Input` 是完整提示量：input + cacheRead + cacheWrite = 1000 + 200 + 0 = 1200。
    for needle in [
        " Total: 3",
        " User: 1",
        " Assistant: 1",
        " Tools: 1 calls, 1 results",
        " Input: 1200",
        " Output: 50",
        " Total: 1250",
    ] {
        assert!(
            screen.contains(needle),
            "/session 面板应有真实统计 {needle:?}（改前全是 0）。屏幕：\n{screen}"
        );
    }
}
