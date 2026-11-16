//! F61/27：压缩后"用量未知"必须显示 `?/<window>`（ref `agent-session.ts:3887-3889`
//! + `footer.ts:150-156`）。
//!
//! 改前：恢复分支在 `resumed_context_tokens == None` 时写 `Some(0.0)`
//! ⇒ 页脚 `0.0%/Nk`，用户会以为上下文被清空了；而实际上"未知"与"0"是两回事。
//!
//! 更隐蔽的一层：恢复时取的是"全文件最后一条可用 assistant 的 usage"，
//! 不区分它在最近一次压缩**之前**还是之后。压缩前的 usage 反映的是压缩前
//! 的上下文大小，用它估算会报出一个远大于实际的百分比。

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

/// 构造一个会话：`user → assistant(usage) → compaction`（压缩后没有新的助手响应）。
fn write_post_compaction_session(path: &std::path::Path) {
    let lines = [
        serde_json::json!({
            "type": "session", "version": 3,
            "id": "01a1a9a8-0000-7000-8000-0000000000c1",
            "timestamp": "2026-01-01T00:00:00.000Z", "cwd": "/tmp"
        }),
        serde_json::json!({
            "type": "message", "id": "u1", "parentId": null,
            "timestamp": "2026-01-01T00:00:01.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "hello"}],
                        "timestamp": "2026-01-01T00:00:01.000Z"}
        }),
        serde_json::json!({
            "type": "message", "id": "a1", "parentId": "u1",
            "timestamp": "2026-01-01T00:00:02.000Z",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "hi"}],
                        "api": "openai-completions", "provider": "p", "model": "m1",
                        "usage": {"input": 1000, "output": 10, "cacheRead": 0, "cacheWrite": 0,
                                  "cost": {"input": 0.0, "output": 0.0, "cacheRead": 0.0,
                                           "cacheWrite": 0.0, "total": 0.0}},
                        "stopReason": "stop"}
        }),
        serde_json::json!({
            "type": "compaction", "id": "c1", "parentId": "a1",
            "timestamp": "2026-01-01T00:00:03.000Z",
            "summary": "sum", "firstKeptEntryId": "u1", "tokensBefore": 1010
        }),
    ];
    let text = lines
        .iter()
        .map(|line| serde_json::to_string(line).expect("序列化"))
        .collect::<Vec<String>>()
        .join("\n");
    std::fs::write(path, format!("{text}\n")).expect("写会话");
}

/// 恢复"压缩后无新助手"的会话 ⇒ 页脚用量是 `?`（不是 `0.0%`、也不是压缩前的百分比）。
#[test]
fn resumed_after_compaction_shows_unknown_usage() {
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
    let session_path = sessions.join("2026-01-01T00-00-00_c1.jsonl");
    write_post_compaction_session(&session_path);

    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2500, "timeoutMs": 90000}},
        {"mark": "resumed"},
        {"text": "\u{4}"},
        {"delayMs": 1200}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("120")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(&raw)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        // **显式恢复这份会话**：不带 `--session` 时 `resumed_path` 为 `None`，
        // 走的是"新会话"分支（页脚 `0.0%` 来自"无用量"而非"未知"），
        // 那样测不到本用例要验证的语义。
        .arg("--session")
        .arg(&session_path)
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
    let footer = screen
        .lines()
        .rfind(|line| line.contains("/200k") || line.contains("(auto)"))
        .unwrap_or_default()
        .to_string();
    assert!(
        footer.contains('?'),
        "压缩后恢复应显示未知用量 `?/200k`。页脚：{footer:?}\n屏幕：\n{screen}"
    );
    assert!(
        !footer.contains("0.0%"),
        "未知用量不应显示成 0.0%。页脚：{footer:?}"
    );
}
