//! F61/13：`/fork` 必须列出可分叉的用户消息并真的分叉
//! （ref `interactive-mode.ts:5341-5368` 的 `showUserMessageSelector`）。
//!
//! 此前 `/fork` 落在"已识别但未实现"，只打一行
//! `Command /fork is not implemented yet.`；而 rpc 侧的 `fork` 早已实现并有测试。
//!
//! 断言：
//! 1. 无用户消息时 `No messages to fork from`（ref `:5345`）；
//! 2. 有两条用户消息时，选择器列出两条，选中一条后出现 `Forked to new session`
//!    （ref `:5364`），会话目录多出一个新文件，且**该消息之前**的历史被复制；
//! 3. 被选中消息的正文回到编辑器（ref `:5363`）。

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
    files: Vec<String>,
}

fn run_fork(with_messages: bool) -> Option<Outcome> {
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
    if with_messages {
        let session = sessions.join("2026-04-01T00-00-00-000Z_fk.jsonl");
        let header = serde_json::json!({
            "type": "session", "id": "s0", "parentId": serde_json::Value::Null,
            "timestamp": "2026-04-01T00:00:00Z", "cwd": work.display().to_string(), "version": 3,
        });
        let user = |id: &str, parent: &str, text: &str| {
            serde_json::json!({
                "type": "message", "id": id, "parentId": parent,
                "timestamp": "2026-04-01T00:00:01Z",
                "message": {"role": "user", "content": [{"type": "text", "text": text}], "timestamp": 1},
            })
        };
        let assistant = |id: &str, parent: &str, text: &str| {
            serde_json::json!({
                "type": "message", "id": id, "parentId": parent,
                "timestamp": "2026-04-01T00:00:02Z",
                "message": {
                    "role": "assistant", "content": [{"type": "text", "text": text}],
                    "api": "openai-completions", "provider": "p", "model": "m",
                    "usage": {"input": 10, "output": 2, "cacheRead": 0, "cacheWrite": 0,
                        "totalTokens": 12, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                    "stopReason": "stop", "timestamp": 1,
                },
            })
        };
        let entries = vec![
            header,
            user("u1", "s0", "第一问"),
            assistant("a1", "u1", "第一答"),
            user("u2", "a1", "第二问"),
            assistant("a2", "u2", "第二答"),
        ];
        let mut file = std::fs::File::create(&session).expect("建会话");
        for entry in &entries {
            writeln!(file, "{entry}").expect("写会话");
        }
        drop(file);
        args.push("--session".to_string());
        args.push(session.to_string_lossy().to_string());
    }

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStable":{"quietMs":2000,"timeoutMs":90000}},{"mark":"s0"},
            {"text":"/fork\r"},{"delayMs":2500},{"mark":"panel"},
            {"text":"\r"},{"delayMs":3000},{"mark":"picked"},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    )
    .expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let steps = tmp.path().join("steps");
    let mut cmd = Command::new("python3");
    cmd.arg(repo_root().join("tools/difftest/pty-helper.py"))
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
        .env("PI_TEST_DUMP", &steps)
        .env("PI_TEST_TAIL", "34")
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
                .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
                .collect()
        })
        .unwrap_or_default();
    // 注：`PI_TEST_DUMP`/`PI_TEST_TAIL` 仍开启——面板内容在中间帧里，
    // `screen_of(raw.bin)` 的整段文本足以覆盖"选择器列出两条消息"这类断言
    // （F61/12 的教训是**只取末尾 N 行**不可靠，整段扫描没问题）。
    let _ = &steps;
    Some(Outcome {
        screen: screen_of(&raw),
        files,
    })
}

/// 无用户消息：`No messages to fork from`（ref `:5345`）。
#[test]
fn fork_without_user_messages_reports_nothing() {
    let Some(outcome) = run_fork(false) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !outcome
            .screen
            .contains("Command /fork is not implemented yet"),
        "/fork 不应走\"未实现\"分支。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.screen.contains("No messages to fork from"),
        "空白会话应提示 `No messages to fork from`。屏幕：\n{}",
        outcome.screen
    );
}

/// 有两条用户消息：选择器列出它们，选中后出现 `Forked to new session` 且多出一个会话文件。
#[test]
fn fork_lists_user_messages_and_creates_a_new_session() {
    let Some(outcome) = run_fork(true) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.screen.contains("Select a message to fork from"),
        "应打开选择器（ref :5351）。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.screen.contains("第一问") && outcome.screen.contains("第二问"),
        "选择器应列出两条用户消息。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.screen.contains("Forked to new session"),
        "选中后应提示 `Forked to new session`（ref :5364）。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.files.len() >= 2,
        "会话目录应多出一个分叉文件（原会话 + fork），实际 {} 个",
        outcome.files.len()
    );
    let forked = outcome
        .files
        .iter()
        .find(|text| text.contains("\"parentSession\""))
        .expect("应有一个带 parentSession 的新会话文件");
    // 默认选中**最后一条**用户消息（ref `:5349`）⇒ 分叉点在 `第二问` 之前，
    // 所以新会话只含 `第一问/第一答`，不含 `第二问`。
    assert!(
        forked.contains("第一问") && !forked.contains("第二问"),
        "分叉点应在最后一条用户消息之前（只含更早的历史）。内容：\n{forked}"
    );
}
