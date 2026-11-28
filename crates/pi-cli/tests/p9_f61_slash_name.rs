//! F61/6：`/name` 必须真的读/写会话显示名（ref `interactive-mode.ts:6408-6430`
//! 的 `handleNameCommand` + `session-manager.ts:1302-1325`）。
//!
//! 三条路径（与 ref 逐条对应）：
//! 1. `/name`（无参）+ 没有名字 ⇒ `Warning: Usage: /name <name>`；
//! 2. `/name <值>` ⇒ 写 `session_info` 条目并显示 `Session name set: <值>`；
//! 3. `/name`（无参）+ 已有名字 ⇒ `Session name: <值>`。
//!
//! 此前 `/name` 落在"已识别但未实现"，只打一行 `Command /name is not implemented yet.`。

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

/// 跑一个按键序列，返回（屏幕、会话文件内容）。
fn run(keys: &str) -> Option<(String, String)> {
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
    // 本地假 provider：用一个**会正常收尾**的 SSE，让首轮产生 assistant 消息 ⇒
    // 会话文件被创建（懒创建语义见下方测试注释）。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定 loopback");
    let port = listener.local_addr().expect("地址").port();
    const SSE: &str = concat!(
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,",
        "\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},",
        "\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,",
        "\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"好的\"},",
        "\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,",
        "\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],",
        "\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}\n\n",
        "data:\n\n"
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            use std::io::Read as _;
            let mut buffer = [0u8; 65536];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                SSE.len(),
                SSE
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    std::fs::write(
        agent.join("models.json"),
        format!(
            r#"{{"providers":{{"p":{{"baseUrl":"http://127.0.0.1:{port}/v1","api":"openai-completions","apiKey":"DUMMY","models":[{{"id":"m","name":"m","contextWindow":200000,"maxTokens":1024}}]}}}}}}"#
        ),
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");

    let keys_path = tmp.path().join("keys.json");
    std::fs::write(&keys_path, keys).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
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
        .arg(&keys_path)
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
    // 会话目录里可能不止一个 jsonl（`/name` 之前那次对话也会建一份）：
    // 取**最大的**那一份（本次会话写入了更多条目），避免读到空文件。
    let session_text = std::fs::read_dir(&sessions)
        .ok()
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
                .filter_map(|entry| {
                    let text = std::fs::read_to_string(entry.path()).ok()?;
                    Some((text.len(), text))
                })
                .max_by_key(|(len, _text)| *len)
                .map(|(_len, text)| text)
                .unwrap_or_default()
        })
        .unwrap_or_default();
    Some((screen_of(&raw), session_text))
}

/// `/name <值>` 必须写入 `session_info` 并给出回执。
///
/// 注：会话文件**懒创建**——ref `_persist`（`session-manager.ts:1160-1188`）在
/// 还没有 assistant 消息时**直接返回**，条目只留在内存。所以先跑一次真实对话把
/// 文件建出来，再发 `/name`；否则读不到（这与 ref 行为一致，不是缺陷）。
#[test]
fn slash_name_writes_session_info() {
    let Some((screen, session)) = run(
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"hi"},{"delayMs":300},{"text":"\r"},{"delayMs":6000},
            {"text":"/name 我的会话"},{"delayMs":600},{"text":"\r"},{"delayMs":3000},{"mark":"set"},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !screen.contains("Command /name is not implemented yet"),
        "/name 不应走\"未实现\"分支。屏幕：\n{screen}"
    );
    assert!(
        screen.contains("Session name set: 我的会话"),
        "/name 应给出 `Session name set: …`（ref :6428）。屏幕：\n{screen}"
    );
    assert!(
        session.contains("\"type\":\"session_info\"") && session.contains("\"name\":\"我的会话\""),
        "会话文件应写入 session_info 条目。会话：\n{session}"
    );
}

/// `/name`（无参）在有名字时显示当前名字，没有时显示用法提示。
#[test]
fn slash_name_without_argument_reports_current_or_usage() {
    let Some((screen, _session)) = run(
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"/name"},{"delayMs":600},{"text":"\r"},{"delayMs":2500},{"mark":"show"},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("Usage: /name <name>") || screen.contains("Session name: "),
        "无参 /name 应显示用法提示或当前名字（ref :6412-6417）。屏幕：\n{screen}"
    );
}
