//! F61/14：`/import <path.jsonl>` 必须把外部会话导入为当前会话
//! （ref `interactive-mode.ts:5591-5640` 的 `handleImportCommand` / `importFromJsonl`）。
//!
//! 此前 `/import` 落在"已识别但未实现"，只打一行
//! `Command /import is not implemented yet.`。
//!
//! 断言：
//! 1. 无参数 ⇒ `Error: Usage: /import <path.jsonl>`（ref `:5594`）；
//! 2. 路径不存在 ⇒ `Failed to import session: …`（ref 的 `SessionImportFileNotFoundError` 分支）；
//! 3. 合法会话 ⇒ `Session imported from: <path>`，会话目录多出一个文件，
//!    且屏幕显示导入会话的内容。

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

/// `import_arg` 为 `None` = 不带参数；否则把该值拼进 `/import` 命令。
fn run_import(import_arg: Option<&str>) -> Option<Outcome> {
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

    // 待导入的外部会话（放在会话目录之外，模拟用户给的路径）。
    let external = tmp.path().join("external.jsonl");
    let header = serde_json::json!({
        "type": "session", "id": "imp", "parentId": serde_json::Value::Null,
        "timestamp": "2026-05-01T00:00:00Z", "cwd": work.display().to_string(), "version": 3,
    });
    let user = serde_json::json!({
        "type": "message", "id": "u1", "parentId": "imp", "timestamp": "2026-05-01T00:00:01Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "导入的提问"}], "timestamp": 1},
    });
    let assistant = serde_json::json!({
        "type": "message", "id": "a1", "parentId": "u1", "timestamp": "2026-05-01T00:00:02Z",
        "message": {
            "role": "assistant", "content": [{"type": "text", "text": "导入的回答"}],
            "api": "openai-completions", "provider": "p", "model": "m",
            "usage": {"input": 10, "output": 2, "cacheRead": 0, "cacheWrite": 0,
                "totalTokens": 12, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": "stop", "timestamp": 1,
        },
    });
    let mut file = std::fs::File::create(&external).expect("建外部会话");
    for entry in [header, user, assistant] {
        writeln!(file, "{entry}").expect("写外部会话");
    }
    drop(file);

    // 命令正文**不含回车**：回车单独作为一步（先 Esc 关补全菜单，再回车提交）。
    // 之前把 `\\r` 拼进字符串，结果编辑器里打的是字面 `\\r` 两个字符——Rust 的
    // `"\\r"` 是反斜杠+r，不是回车。
    let command = match import_arg {
        Some(arg) => format!("/import {arg}"),
        None => "/import".to_string(),
    };
    let keys = tmp.path().join("keys.json");
    // 全部转义交给 `serde_json`：手写 `\u001b`/`\r` 在 `format!` 里会被当成真实控制字符，
    // 生成的 JSON 不合法（pty-helper 解析失败会让进程一直不静止，实测跑满 300s）。
    let script = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"mark": "s0"},
        {"text": command},
        {"delayMs": 600},
        {"text": "\u{1b}"},
        {"delayMs": 300},
        {"text": "\r"},
        {"delayMs": 3000},
        {"mark": "imported"},
        {"text": "\u{4}"},
        {"delayMs": 1500}
    ]);
    std::fs::write(&keys, serde_json::to_string(&script).expect("序列化")).expect("写 keys");
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
                .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
                .collect()
        })
        .unwrap_or_default();
    Some(Outcome {
        screen: screen_of(&raw),
        files,
    })
}

/// 无参数 ⇒ 用法提示（ref `:5594`）。
#[test]
fn import_without_argument_shows_usage() {
    let Some(outcome) = run_import(None) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !outcome
            .screen
            .contains("Command /import is not implemented yet"),
        "/import 不应走\"未实现\"分支。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.screen.contains("Usage: /import <path.jsonl>"),
        "无参数应提示用法。屏幕：\n{}",
        outcome.screen
    );
}

/// 路径不存在 ⇒ 失败提示。
#[test]
fn import_missing_file_reports_failure() {
    let Some(outcome) = run_import(Some("/nonexistent/nope.jsonl")) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.screen.contains("Failed to import session"),
        "不存在的路径应报失败。屏幕：\n{}",
        outcome.screen
    );
}

/// 合法会话 ⇒ 导入成功，多出一个会话文件，屏幕显示其内容。
#[test]
fn import_valid_session_switches_to_it() {
    let Some(dir) = Some(tempfile::tempdir().expect("临时目录")) else {
        return;
    };
    let external = dir.path().join("good.jsonl");
    let mut file = std::fs::File::create(&external).expect("建文件");
    let header = serde_json::json!({
        "type": "session", "id": "imp", "parentId": serde_json::Value::Null,
        "timestamp": "2026-05-01T00:00:00Z", "cwd": "/tmp", "version": 3,
    });
    let user = serde_json::json!({
        "type": "message", "id": "u1", "parentId": "imp", "timestamp": "2026-05-01T00:00:01Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "导入的提问"}], "timestamp": 1},
    });
    writeln!(file, "{header}").expect("写");
    writeln!(file, "{user}").expect("写");
    drop(file);
    let Some(outcome) = run_import(Some(external.to_string_lossy().as_ref())) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.screen.contains("Session imported from:"),
        "应提示导入成功（ref :5609）。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        !outcome.files.is_empty(),
        "会话目录里应出现导入后的会话文件"
    );
    assert!(
        outcome.screen.contains("导入的提问"),
        "屏幕应显示导入会话的内容。屏幕：\n{}",
        outcome.screen
    );
}
