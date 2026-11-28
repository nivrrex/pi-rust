//! F60/1d：`/allcopy` 必须在**真实扩展宿主加载路径**下端到端可用。
//!
//! 用户报的故障是 `/allcopy` 报 `require is not defined`。根因是 shim
//! （`tools/ext-host/shim/pi-coding-agent.mjs`，ESM `.mjs`）内部用了
//! `require("node:child_process")` / `require("node:fs")`，而真实宿主
//! `tools/ext-host/runner.mjs` 用 `await import()` 加载扩展、**从不**注入 CommonJS 垫片。
//!
//! F60/1b 已把单测改成走同一条加载路径（`shim-clipboard.test.mjs`，不预置
//! `globalThis.require`）。但单测只覆盖 shim 自身，**不证明**整条链路
//! （TUI → 扩展命令 → `copyToClipboard` → 剪贴板命令）在真实宿主里跑得通。
//!
//! 本用例补上这一层：真 pty + 真实 `pi-handoff-gate` + 真实会话，输入 `/allcopy`，
//! 断言屏幕出现成功回执且**不出现** `require is not defined`。

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

fn copy_dir(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let to = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// 造一个只含假剪贴板命令的 `PATH` 目录（`xclip` 把 stdin 落到文件）。
///
/// 这样测的是**真实 spawn 路径**：`copyToClipboard` 真的 fork 了一个子进程、
/// 真的把文本写进了它的 stdin（不是 mock 内部函数）。
fn fake_clipboard_bin(dir: &Path, sink: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let script = format!("#!/bin/sh\ncat > '{}'\nexit 0\n", sink.display());
    let path = dir.join("xclip");
    std::fs::write(&path, script)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
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
    /// 假 `xclip` 收到的字节（`None` = 文件没被写过）。
    clipboard: Option<String>,
    exit_code: Option<i32>,
}

/// 真 pty 里跑 `pi-rs --session <合成会话>`，输入 `/allcopy` 回车，再 Ctrl+D。
fn run_allcopy() -> Option<Outcome> {
    let node = node_path()?;
    let extension_src = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".pi/agent/extensions/pi-handoff-gate"))
        .filter(|path| path.is_dir())?;

    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    for dir in [&home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::create_dir_all(agent.join("extensions")).expect("建扩展目录");
    copy_dir(&extension_src, &agent.join("extensions/pi-handoff-gate")).expect("拷贝扩展");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    // 端口 1：本用例不发起模型请求（只跑扩展命令），连不上也无所谓。
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

    // 合成会话：一条 user + 一条 assistant（`buildAllcopyText` 要有内容可复制）。
    let session_path = sessions.join("2026-01-01T00-00-00-000Z_ac.jsonl");
    let header = serde_json::json!({
        "type": "session", "id": "s0", "parentId": serde_json::Value::Null,
        "timestamp": "2026-01-01T00:00:00Z", "cwd": work.display().to_string(), "version": 3,
    });
    let user = serde_json::json!({
        "type": "message", "id": "u1", "parentId": "s0", "timestamp": "2026-01-01T00:00:01Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "做点事"}]},
    });
    let assistant = serde_json::json!({
        "type": "message", "id": "a1", "parentId": "u1", "timestamp": "2026-01-01T00:00:02Z",
        "message": {
            "role": "assistant",
            "content": [{"type": "text", "text": "这是要复制到剪贴板的回答正文。"}],
            "api": "openai-completions", "provider": "p", "model": "m",
            "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0,
                "totalTokens": 2, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": "stop", "timestamp": 0,
        },
    });
    let mut file = std::fs::File::create(&session_path).expect("建会话文件");
    for entry in [header, user, assistant] {
        writeln!(file, "{entry}").expect("写会话");
    }
    drop(file);

    // 假剪贴板命令 + 只含它的 PATH（加上 pi-rs 需要的最小系统路径）。
    let bin_dir = tmp.path().join("bin");
    let clipboard_sink = tmp.path().join("clipboard.txt");
    fake_clipboard_bin(&bin_dir, &clipboard_sink).expect("造假 xclip");

    let keys_path = tmp.path().join("keys.json");
    std::fs::write(
        &keys_path,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"/allcopy"},{"delayMs":600},{"text":"\r"},{"delayMs":5000},{"mark":"after"},
            {"text":"\u0004"},{"delayMs":1500},{"mark":"exit"}]"#,
    )
    .expect("写 keys");

    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let system_path = std::env::var("PATH").unwrap_or_default();
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("36")
        .arg("--exit-code-file")
        .arg(&code_path)
        .arg("--out")
        .arg(&raw_path)
        .arg("--script")
        .arg(&keys_path)
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
        // 假 xclip 排在最前：`copyToClipboard` 会优先用它。
        .env("PATH", format!("{}:{}", bin_dir.display(), system_path))
        // 有 DISPLAY 才会走 X11 命令分支（而不是 OSC 52）——正是用户真机的场景。
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

    let clipboard = std::fs::read_to_string(&clipboard_sink).ok();
    let exit_code = std::fs::read_to_string(&code_path)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok());
    Some(Outcome {
        screen: screen_of(&raw_path),
        clipboard,
        exit_code,
    })
}

/// **核心回归**：`/allcopy` 在真实宿主路径下不报 `require is not defined`。
///
/// 改前（shim 内用 `require`）这里会出现 `Error: require is not defined`，
/// 且没有任何复制结果。
#[test]
fn allcopy_works_through_the_real_host() {
    let Some(outcome) = run_allcopy() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    assert!(
        !outcome.screen.contains("require is not defined"),
        "/allcopy 报 require is not defined（F60/1 的原始故障）。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.screen.contains("已复制到剪贴板"),
        "/allcopy 应给出成功回执。屏幕：\n{}",
        outcome.screen
    );
}

/// 复制的内容必须真的到达剪贴板命令（不是"只打了句成功提示"）。
#[test]
fn allcopy_actually_writes_to_the_clipboard_command() {
    let Some(outcome) = run_allcopy() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    let clipboard = outcome.clipboard.unwrap_or_default();
    assert!(
        clipboard.contains("这是要复制到剪贴板的回答正文。"),
        "假 xclip 没有收到正文（copyToClipboard 没真的 spawn 或没写 stdin）。收到：{clipboard:?}"
    );
}

/// `/allcopy` 之后进程仍能正常退出（命令没有把主循环卡住）。
#[test]
fn allcopy_leaves_the_process_responsive() {
    let Some(outcome) = run_allcopy() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "/allcopy 之后 Ctrl+D 应正常退出。屏幕：\n{}",
        outcome.screen
    );
}
