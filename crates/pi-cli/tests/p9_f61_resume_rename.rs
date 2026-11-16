//! F61/31：`/resume` 面板的 `ctrl+r` 重命名（ref `session-selector.ts:885-937`）。
//!
//! 改前：只实现了"进入重命名模式"（`enter_rename`），没有输入/保存/取消的分派
//! ⇒ `ctrl+r` 之后敲字不进输入框、回车也不落盘，界面上却是个看起来可用的输入框。
//!
//! ref 语义：Enter 保存（`value.trim()`，空值**不保存**并留在模式内）、Esc 取消、
//! 保存成功后刷新列表（`:933-937` 的 `refreshSessionsAfterMutation`）。

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

/// 造一份可恢复的会话。
///
/// `cwd` 必须等于**工作目录**：`/resume` 的 `Current Folder` 作用域按会话 header 的
/// `cwd` 过滤（`scan_sessions`），写成别的路径会让面板显示
/// `No sessions in current folder`（实测踩过），于是 `selected_path()` 为 `None`、
/// 重命名保存不落盘。
fn write_session(path: &std::path::Path, cwd: &std::path::Path) {
    let lines = [
        serde_json::json!({
            "type": "session", "version": 3,
            "id": "01a1a9a8-0000-7000-8000-0000000000r1".replace('r', "1"),
            "timestamp": "2026-01-01T00:00:00.000Z",
            "cwd": cwd.to_string_lossy().to_string()
        }),
        serde_json::json!({
            "type": "message", "id": "u1", "parentId": null,
            "timestamp": "2026-01-01T00:00:01.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "hello"}],
                        "timestamp": "2026-01-01T00:00:01.000Z"}
        }),
    ];
    let text = lines
        .iter()
        .map(|line| serde_json::to_string(line).expect("序列化"))
        .collect::<Vec<String>>()
        .join("\n");
    std::fs::write(path, format!("{text}\n")).expect("写会话");
}

/// `ctrl+r` 进入重命名 → 输入 `renamed` → Enter 保存 ⇒ 会话文件里出现 `session_info`。
#[test]
fn resume_rename_saves_the_new_name() {
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
    let session_path = sessions.join("2026-01-01T00-00-00_r1.jsonl");
    write_session(&session_path, &work);

    // `/resume` → Esc 关补全 → Enter → `ctrl+r` → 输入 → Enter → Esc → 退出
    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"text": "/resume"},
        {"delayMs": 700},
        {"text": "\u{1b}"},
        {"delayMs": 300},
        {"text": "\r"},
        {"delayMs": 2500},
        {"mark": "list"},
        {"text": "\u{12}"},
        {"delayMs": 800},
        {"mark": "rename"},
        // 逐字符发送：整串一次性写入时，终端把它们当成一次 "paste"，
        // 重命名输入框只认逐个按键事件（实测整串输入后文件里没有 session_info）。
        {"text": "r"}, {"delayMs": 80},
        {"text": "e"}, {"delayMs": 80},
        {"text": "n"}, {"delayMs": 80},
        {"text": "a"}, {"delayMs": 80},
        {"text": "m"}, {"delayMs": 80},
        {"text": "e"}, {"delayMs": 80},
        {"text": "d"}, {"delayMs": 80},
        {"text": "\r"},
        {"delayMs": 1500},
        {"text": "\u{1b}"},
        {"delayMs": 500},
        {"text": "\u{4}"},
        {"delayMs": 1200}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("32")
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
        screen.contains("Rename Session"),
        "`ctrl+r` 应进入重命名面板。屏幕：\n{screen}"
    );
    let text = std::fs::read_to_string(&session_path).expect("读会话");
    assert!(
        text.contains("\"session_info\""),
        "保存后会话文件里应有 session_info 条目。文件：\n{text}"
    );
    assert!(
        text.contains("\"renamed\""),
        "保存的名字应是输入的内容。文件：\n{text}"
    );
}

/// 空名字不保存（ref `:912` 的 `if (!next) return`）：直接回车不应写 `session_info`。
#[test]
fn resume_rename_ignores_an_empty_name() {
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
    let session_path = sessions.join("2026-01-01T00-00-00_r2.jsonl");
    write_session(&session_path, &work);

    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"text": "/resume"},
        {"delayMs": 700},
        {"text": "\u{1b}"},
        {"delayMs": 300},
        {"text": "\r"},
        {"delayMs": 2500},
        {"text": "\u{12}"},
        {"delayMs": 800},
        {"text": "\r"},
        {"delayMs": 1200},
        {"text": "\u{1b}"},
        {"delayMs": 500},
        {"text": "\u{4}"},
        {"delayMs": 1200}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("32")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
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
    let text = std::fs::read_to_string(&session_path).expect("读会话");
    assert!(
        !text.contains("\"session_info\""),
        "空名字不应写入 session_info（ref `:912` 的 `if (!next) return`）。文件：\n{text}"
    );
}

/// `ctrl+backspace`（`app.session.deleteNoninvasive`）在 `/resume` 面板里
/// 应进入与 `ctrl+d` 相同的删除确认态（ref `session-selector.ts:599-608`）。
///
/// 改前：该键没有任何分派（`/resume` 面板内被当成无关按键吞掉），
/// 用户按 ref 的提示按键得不到任何反应。
#[test]
fn resume_ctrl_backspace_enters_delete_confirmation() {
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
    let session_path = sessions.join("2026-01-01T00-00-00_r3.jsonl");
    write_session(&session_path, &work);

    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"text": "/resume"},
        {"delayMs": 700},
        {"text": "\u{1b}"},
        {"delayMs": 300},
        {"text": "\r"},
        {"delayMs": 2500},
        {"mark": "list"},
        {"text": "\u{1b}[127;5u"},
        {"delayMs": 1200},
        {"mark": "confirm"},
        {"text": "\u{1b}"},
        {"delayMs": 500},
        {"text": "\u{4}"},
        {"delayMs": 1200}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("32")
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
        screen.contains("Delete session?"),
        "`ctrl+backspace` 应进入删除确认态（ref `session-selector.ts:599-608`）。屏幕：\n{screen}"
    );
}
