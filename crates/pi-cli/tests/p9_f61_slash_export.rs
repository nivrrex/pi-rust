//! F61/15：`/export <path.jsonl>` 必须把**当前分支**导出成 JSONL
//! （ref `interactive-mode.ts:5593-5607` → `session-export.ts:32-45`）。
//!
//! 断言：
//! 1. 非 `.jsonl` 目标 ⇒ 明确报错（HTML 导出未实现，不假装成功）；
//! 2. `.jsonl` 目标 ⇒ `Session exported to: <path>`，文件存在，
//!    首行是会话头，且**只含当前分支**（不含被离开的旧分支）。

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
    /// 导出目标文件的内容（若存在）。
    exported: Option<String>,
}

fn run_export(target_name: &str) -> Option<Outcome> {
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

    // 会话：一条主线 + 一条**旧分支**（`orphan`，parent 指向 s0 而非当前叶子）。
    // 导出只应包含当前分支，`orphan` 不应出现。
    let session = sessions.join("2026-06-01T00-00-00-000Z_ex.jsonl");
    let header = serde_json::json!({
        "type": "session", "id": "s0", "parentId": serde_json::Value::Null,
        "timestamp": "2026-06-01T00:00:00Z", "cwd": work.display().to_string(), "version": 3,
    });
    let user = serde_json::json!({
        "type": "message", "id": "u1", "parentId": "s0", "timestamp": "2026-06-01T00:00:01Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "主线提问"}], "timestamp": 1},
    });
    let assistant = serde_json::json!({
        "type": "message", "id": "a1", "parentId": "u1", "timestamp": "2026-06-01T00:00:02Z",
        "message": {
            "role": "assistant", "content": [{"type": "text", "text": "主线回答"}],
            "api": "openai-completions", "provider": "p", "model": "m",
            "usage": {"input": 10, "output": 2, "cacheRead": 0, "cacheWrite": 0,
                "totalTokens": 12, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": "stop", "timestamp": 1,
        },
    });
    let orphan = serde_json::json!({
        "type": "message", "id": "o1", "parentId": "s0", "timestamp": "2026-06-01T00:00:03Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "旧分支的内容"}], "timestamp": 1},
    });
    // **顺序很重要**：会话是**追加写**的，文件的最后一条就是当前叶子（ref 的
    // `SessionManager.leafId` 在加载后取路径末尾）。因此"旧分支"必须写在**前面**、
    // 主线写在**后面**——反过来（把 orphan 放最后）会让人误以为叶子取错了。
    let mut file = std::fs::File::create(&session).expect("建会话");
    for entry in [header, orphan, user, assistant] {
        writeln!(file, "{entry}").expect("写会话");
    }
    drop(file);

    let target = work.join(target_name);
    let command = format!("/export {}", target.display());
    let keys = tmp.path().join("keys.json");
    let script = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"mark": "s0"},
        {"text": command},
        {"delayMs": 600},
        {"text": "\u{1b}"},
        {"delayMs": 300},
        {"text": "\r"},
        {"delayMs": 3000},
        {"mark": "exported"},
        {"text": "\u{4}"},
        {"delayMs": 1500}
    ]);
    std::fs::write(&keys, serde_json::to_string(&script).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("120")
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
    Some(Outcome {
        screen: screen_of(&raw),
        exported: std::fs::read_to_string(&target).ok(),
    })
}

/// `.jsonl` 目标：导出成功，文件含当前分支、不含旧分支。
#[test]
fn export_jsonl_writes_the_current_branch() {
    let Some(outcome) = run_export("out.jsonl") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !outcome
            .screen
            .contains("Command /export is not implemented yet"),
        "/export 不应走\"未实现\"分支。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.screen.contains("Session exported to:"),
        "应提示导出成功（ref :5601）。屏幕：\n{}",
        outcome.screen
    );
    let exported = outcome.exported.expect("导出文件应存在");
    assert!(
        exported.contains("\"type\":\"session\""),
        "首行应是会话头。内容：\n{exported}"
    );
    assert!(
        exported.contains("主线提问") && exported.contains("主线回答"),
        "导出应含当前分支的消息。内容：\n{exported}"
    );
    assert!(
        !exported.contains("旧分支的内容"),
        "导出**只含当前分支**，不应包含被离开的旧分支（ref serializeSessionBranch）。内容：\n{exported}"
    );
}

/// 非 `.jsonl` 目标：明确报错（HTML 导出未实现），不假装成功。
#[test]
fn export_html_target_reports_not_implemented() {
    let Some(outcome) = run_export("out.html") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.screen.contains("Failed to export session"),
        "HTML 目标应明确报错。屏幕：\n{}",
        outcome.screen
    );
    assert!(outcome.exported.is_none(), "不应写出任何文件（不假装成功）");
}
