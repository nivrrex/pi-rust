//! F61/8：`/clone` 必须复制当前分支到新会话（ref `interactive-mode.ts:5379-5398`
//! 的 `handleCloneCommand` → `runtimeHost.fork(leafId, {position:"at"})`）。
//!
//! 此前 `/clone` 落在"已识别但未实现"，只打一行
//! `Command /clone is not implemented yet.`；而 rpc 侧的 `clone` 早已实现并有测试，
//! TUI 只差接线。
//!
//! 断言：
//! 1. 无条目时 `Nothing to clone yet`（ref `:5382`）；
//! 2. 有会话时出现 `Cloned to new session`（ref `:5394`），且会话目录里**多出一个** jsonl；
//! 3. 新文件是**当前分支的复制**（条目 type 序列 ⊆ 原会话，且带 `parentSession` 头）。

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

struct Outcome {
    screen: String,
    /// 会话目录里所有 jsonl（含新克隆出来的）。
    files: Vec<(String, String)>,
}

fn run_clone(with_session: bool) -> Option<Outcome> {
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

    let mut args: Vec<String> = Vec::new();
    if with_session {
        let session = sessions.join("2026-01-01T00-00-00-000Z_cl.jsonl");
        let header = serde_json::json!({
            "type": "session", "id": "s0", "parentId": serde_json::Value::Null,
            "timestamp": "2026-01-01T00:00:00Z", "cwd": work.display().to_string(), "version": 3,
        });
        let user = serde_json::json!({
            "type": "message", "id": "u1", "parentId": "s0", "timestamp": "2026-01-01T00:00:01Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "问"}], "timestamp": 1},
        });
        let assistant = serde_json::json!({
            "type": "message", "id": "a1", "parentId": "u1", "timestamp": "2026-01-01T00:00:02Z",
            "message": {
                "role": "assistant", "content": [{"type": "text", "text": "答"}],
                "api": "openai-completions", "provider": "p", "model": "m",
                "usage": {"input": 10, "output": 2, "cacheRead": 0, "cacheWrite": 0,
                    "totalTokens": 12, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                "stopReason": "stop", "timestamp": 2,
            },
        });
        let mut file = std::fs::File::create(&session).expect("建会话");
        for entry in [header, user, assistant] {
            writeln!(file, "{entry}").expect("写会话");
        }
        drop(file);
        args.push("--session".to_string());
        args.push(session.to_string_lossy().to_string());
    }

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"/clone"},{"delayMs":600},{"text":"\r"},{"delayMs":600},{"text":"\r"},{"delayMs":4000},{"mark":"clone"},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    )
    .expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let mut cmd = Command::new("python3");
    cmd.arg(repo_root().join("tools/difftest/pty-helper.py"))
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
        .arg(env!("CARGO_BIN_EXE_pi-rs"));
    for arg in &args {
        cmd.arg(arg);
    }
    let output = cmd
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
    let files = std::fs::read_dir(&sessions)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
                .map(|entry| {
                    (
                        entry.file_name().to_string_lossy().to_string(),
                        std::fs::read_to_string(entry.path()).unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Outcome {
        screen: screen_of(&raw),
        files,
    })
}

/// 空白会话：`/clone` 给 `Nothing to clone yet`（ref `:5382`）。
#[test]
fn clone_on_empty_session_reports_nothing_to_clone() {
    let Some(outcome) = run_clone(false) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !outcome
            .screen
            .contains("Command /clone is not implemented yet"),
        "/clone 不应走\"未实现\"分支。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.screen.contains("Nothing to clone yet"),
        "空白会话应提示 `Nothing to clone yet`。屏幕：\n{}",
        outcome.screen
    );
}

/// 有会话：`/clone` 给 `Cloned to new session`，并真的多出一个会话文件。
#[test]
fn clone_copies_the_current_branch_to_a_new_session() {
    let Some(outcome) = run_clone(true) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.screen.contains("Cloned to new session"),
        "/clone 应提示 `Cloned to new session`（ref :5394）。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.files.len() >= 2,
        "会话目录应多出一个文件（原会话 + 克隆），实际 {} 个",
        outcome.files.len()
    );
    // 新文件：头里带 `parentSession`，且含有原会话的两条消息。
    let cloned = outcome
        .files
        .iter()
        .find(|(_name, text)| text.contains("\"parentSession\""))
        .map(|(_name, text)| text.clone())
        .expect("应有一个带 parentSession 的新会话文件");
    assert!(
        cloned.contains("\"text\":\"问\"") && cloned.contains("\"text\":\"答\""),
        "克隆出的会话应复制当前分支的消息。内容：\n{cloned}"
    );
}
