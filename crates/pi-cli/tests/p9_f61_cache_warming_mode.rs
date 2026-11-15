//! F61/11：`/session` 面板的 `Cache Warming: Mode:` 必须读**真实设置**
//! （ref `interactive-mode.ts:6474` → `settingsManager.getCacheWarmingMode()`
//! `settings-manager.ts:955-958`）。
//!
//! 此前该行是硬编码 `Mode: streaming`——用户在 settings.json 里写 `"cacheWarming": "off"`
//! 也仍显示 `streaming`。
//!
//! 同时锁定 ref 的**合法性校验**：只认 `CACHE_WARMING_MODES`
//! （`settings-manager.ts:77` 的 `["off","streaming","idle"]`），
//! 非法值回落 `"streaming"`。

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

/// 用给定的 `cacheWarming` 值跑 `/session`，返回屏幕。
fn run_panel(cache_warming: Option<&str>) -> Option<String> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    for dir in [&agent, &home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    let mut settings = serde_json::json!({
        "theme": "dark", "defaultProvider": "p", "defaultModel": "m"
    });
    if let Some(value) = cache_warming {
        settings["cacheWarming"] = serde_json::Value::String(value.to_string());
    }
    std::fs::write(
        agent.join("settings.json"),
        serde_json::to_string(&settings).expect("序列化"),
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

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"/session\r"},{"delayMs":3000},{"mark":"panel"},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    )
    .expect("写 keys");
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
    let _ = std::io::stdout().flush();
    Some(screen_of(&raw))
}

/// settings 写 `off` ⇒ 面板显示 `Mode: off`（改前恒为 `streaming`）。
#[test]
fn cache_warming_mode_reflects_settings() {
    let Some(screen) = run_panel(Some("off")) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains(" Mode: off"),
        "settings 的 cacheWarming=off 应显示在面板里。屏幕：\n{screen}"
    );
    assert!(
        !screen.contains(" Mode: streaming"),
        "不应再显示硬编码的 streaming。屏幕：\n{screen}"
    );
}

/// 非法值 ⇒ 回落 `streaming`（ref `settings-manager.ts:957` 的 `includes` 校验）。
#[test]
fn unknown_cache_warming_mode_falls_back_to_streaming() {
    let Some(screen) = run_panel(Some("bogus")) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains(" Mode: streaming"),
        "非法值应回落 streaming。屏幕：\n{screen}"
    );
}
