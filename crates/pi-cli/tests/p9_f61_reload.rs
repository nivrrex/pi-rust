//! F61/22：`/reload`（ref `interactive-mode.ts:6170-6250`）。
//!
//! 语义：重载 keybindings/extensions/skills/prompts/themes/context files。
//! 关键约束来自 ESM：扩展模块在同一 Node 进程里 `import` 后进缓存，**再 import
//! 不会重新执行模块顶层**（扩展的所有注册都在顶层）⇒ 只能换进程。
//! 因此 `reload_host` 必须能替换进程级 `HOST` 槽位（此前是 `OnceLock<Option<…>>`，
//! 换不掉，这也正是 `/reload` 长时间未实现的真正原因）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// `/reload` 不应再走"未实现"分支，并给出 ref 的状态行原文。
#[test]
fn reload_command_reports_the_ref_status_line() {
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

    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"text": "/reload"},
        {"delayMs": 600},
        {"text": "\u{1b}"},
        {"delayMs": 300},
        {"text": "\r"},
        {"delayMs": 8000},
        {"mark": "after"},
        {"text": "\u{4}"},
        {"delayMs": 1500}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("120")
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
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("运行 pty-helper");
    assert!(
        output.status.success(),
        "pty-helper 应成功（stderr：{}）",
        String::from_utf8_lossy(&output.stderr)
    );
    let screen = screen_of(&raw);
    assert!(
        !screen.contains("Command /reload is not implemented yet"),
        "/reload 不应再走未实现分支。屏幕：\n{screen}"
    );
    assert!(
        screen.contains(
            "Reloaded keybindings, extensions, skills, prompts, themes, and context files"
        ),
        "应给出 ref 的状态行原文（interactive-mode.ts:6243-6246）。屏幕：\n{screen}"
    );
}

/// `reload_host` 必须**真的换掉**扩展宿主进程（PID 变化）。
///
/// 为什么用 PID 判定：`/reload` 的全部价值在于"让扩展模块重新求值"。
/// ESM 在同一 Node 进程里 `import` 后进缓存，重新 import 不会重新执行模块顶层
/// （扩展的所有注册都在顶层）。所以"PID 没变"就等于"什么都没重载"，
/// 只是打印了一行成功状态——那是最坏的假成功。
#[test]
fn reload_host_replaces_the_host_process() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    let ext = agent.path().join("extensions/probe");
    std::fs::create_dir_all(&ext).expect("建目录");
    std::fs::write(
        ext.join("index.mjs"),
        "export default function probe(pi) { pi.on(\"session_start\", () => {}); }\n",
    )
    .expect("写扩展");

    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", &node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    // 首次启动宿主。
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    let first = pi_cli::ext_tools::host_pid_for_test();
    let Some(first_pid) = first else {
        eprintln!("跳过：本机宿主未启动（无 node 或扩展不可用）");
        return;
    };

    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("reload 应成功");
    let second = pi_cli::ext_tools::host_pid_for_test();
    let Some(second_pid) = second else {
        panic!("reload 之后应有新宿主（reload 前 PID={first_pid}）");
    };
    assert_ne!(
        first_pid, second_pid,
        "reload 必须换掉宿主进程，否则扩展模块不会被重新求值（PID 仍是 {first_pid}）"
    );

    // 清理新宿主，避免留下孤儿 node 进程。
    pi_cli::ext_tools::shutdown_host(std::time::Duration::from_secs(3));
}
