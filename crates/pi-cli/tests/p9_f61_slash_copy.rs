//! F61/5：`/copy` 必须真的复制最后一条助手消息（ref `interactive-mode.ts:3126-3129`
//! → `handleCopyCommand()` `:6377-6407`）。
//!
//! 此前 `/copy` 落在"已识别但未实现"分支，只打一行
//! `Command /copy is not implemented yet.` —— 而它与 `ctrl+x`（`app.message.copy`）
//! 是**同一段语义**（复制最后一条助手消息），实现成本几乎为零。
//!
//! 本用例走真 pty + 假 `xclip`（真实 spawn 与真实 stdin），断言：
//! 1. 屏幕出现 `Copied last agent message to clipboard`（不是"未实现"）；
//! 2. 假 `xclip` 收到的是那条助手消息正文。

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
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
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
        out.push(ch);
    }
    out
}

struct Outcome {
    screen: String,
    clipboard: Option<String>,
}

fn run_copy() -> Option<Outcome> {
    let node = node_path()?;
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

    // 合成会话：一条助手消息（`getLastAssistantText` 的输入）。
    let session_path = sessions.join("2026-01-01T00-00-00-000Z_cp.jsonl");
    let header = serde_json::json!({
        "type": "session", "id": "s0", "parentId": serde_json::Value::Null,
        "timestamp": "2026-01-01T00:00:00Z", "cwd": work.display().to_string(), "version": 3,
    });
    let user = serde_json::json!({
        "type": "message", "id": "u1", "parentId": "s0", "timestamp": "2026-01-01T00:00:01Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "问"}]},
    });
    let assistant = serde_json::json!({
        "type": "message", "id": "a1", "parentId": "u1", "timestamp": "2026-01-01T00:00:02Z",
        "message": {
            "role": "assistant",
            "content": [{"type": "text", "text": "要被复制的回答正文"}],
            "api": "openai-completions", "provider": "p", "model": "m",
            "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0,
                "totalTokens": 2, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": "stop", "timestamp": 0,
        },
    });
    let mut file = std::fs::File::create(&session_path).expect("建会话");
    for entry in [header, user, assistant] {
        writeln!(file, "{entry}").expect("写会话");
    }
    drop(file);

    // 假 xclip：把 stdin 落到文件。
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("建 bin");
    let sink = tmp.path().join("clipboard.txt");
    let script = format!("#!/bin/sh\ncat > '{}'\nexit 0\n", sink.display());
    let xclip = bin.join("xclip");
    std::fs::write(&xclip, script).expect("写假 xclip");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&xclip, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"/copy"},{"delayMs":600},{"text":"\r"},{"delayMs":4000},{"mark":"after"},
            {"text":"\u0004"},{"delayMs":1500},{"mark":"exit"}]"#,
    )
    .expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let system_path = std::env::var("PATH").unwrap_or_default();
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(&code)
        .arg("--out")
        .arg(&raw)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--session")
        .arg(&session_path)
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PATH", format!("{}:{}", bin.display(), system_path))
        .env("DISPLAY", ":0")
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
    Some(Outcome {
        screen: screen_of(&raw),
        clipboard: std::fs::read_to_string(&sink).ok(),
    })
}

/// 两段断言放同一个用例（`PI_EXT_TEST_LOG`/`PATH` 等是进程级环境变量）。
#[test]
fn slash_copy_copies_the_last_assistant_message() {
    let Some(outcome) = run_copy() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !outcome
            .screen
            .contains("Command /copy is not implemented yet"),
        "/copy 不应再走\"未实现\"分支。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome
            .screen
            .contains("Copied last agent message to clipboard"),
        "/copy 应给出成功回执（ref :6401）。屏幕：\n{}",
        outcome.screen
    );
    let clipboard = outcome.clipboard.unwrap_or_default();
    assert!(
        clipboard.contains("要被复制的回答正文"),
        "剪贴板里应是最后一条助手消息正文，实际：{clipboard:?}"
    );
}
