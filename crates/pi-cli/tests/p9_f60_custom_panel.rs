//! F60/11：`ctx.ui.custom` 的自定义面板在 TUI 里必须可用（渲染 + 按键 + 关闭）。
//!
//! 依赖它的已装扩展：`pi-interact`（`ask_user` 的多选/单选面板，
//! `index.ts:133-212`）与 `pi-secrets`（`request_secret` 的输入框）。
//! 都是 `ctx.ui.custom((tui, theme, keybindings, done) => component)` 形态：
//! 宿主渲染 `component.render(width)` 的行，把按键经 `ext.custom.input` 转给
//! `component.handleInput(data)`，组件调 `done(value)` 后收起。
//!
//! 本用例走**真实 pty**：桌面级按键（↓、Enter、Esc）真的从 pty 进主循环，
//! 断言面板出现 → 选中项随 ↓ 移动 → Enter 后 `done(value)` 的结果被扩展拿到。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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

fn install(agent: &Path) {
    let target = agent.join("extensions/custom");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/custom-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
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
    log: String,
}

/// 真 pty：`session_start` 时扩展打开面板 → 屏幕出现两项 → ↓ 移动 → Enter 选中 beta。
fn run_custom_panel(keys: &str) -> Option<Outcome> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    for dir in [&home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    install(&agent);
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    // 端口 1：本用例不发起模型请求。
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

    let log_path = tmp.path().join("probe.log");
    let keys_path = tmp.path().join("keys.json");
    std::fs::write(&keys_path, keys).expect("写 keys");
    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(&code_path)
        .arg("--out")
        .arg(&raw_path)
        .arg("--script")
        .arg(&keys_path)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("PI_NODE", &node)
        .env("PI_EXT_TEST_LOG", &log_path)
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
        screen: screen_of(&raw_path),
        log: std::fs::read_to_string(&log_path).unwrap_or_default(),
    })
}

/// 面板必须**渲染出来**（两行选项 + 选中标记）。
#[test]
fn custom_panel_renders_its_lines() {
    let Some(outcome) = run_custom_panel(
        r#"[{"waitStable":{"quietMs":2500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"\u001b"},{"delayMs":1200},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.screen.contains("→ alpha"),
        "自定义面板应渲染选中项（`→ alpha`）。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.screen.contains("  beta"),
        "自定义面板应渲染第二项（`  beta`）。屏幕：\n{}",
        outcome.screen
    );
}

/// 按键必须转发给组件（↓ 之后选中项变成 beta），且 Enter 让 `done(value)` 带出 beta。
#[test]
fn custom_panel_handles_keys_and_returns_the_result() {
    let Some(outcome) = run_custom_panel(
        r#"[{"waitStable":{"quietMs":2500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"\u001b[B"},{"delayMs":1200},{"mark":"down"},
            {"text":"\r"},{"delayMs":1500},{"mark":"enter"},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.screen.contains("→ beta"),
        "↓ 之后选中项应移到 beta（按键真的转给了组件）。屏幕：\n{}",
        outcome.screen
    );
    assert!(
        outcome.log.contains("custom:result:beta"),
        "Enter 之后组件调 `done(\"beta\")` 的结果应回到扩展。日志：\n{}",
        outcome.log
    );
}

/// Esc 关闭面板并回传 `undefined`（ref：组件自行 `done(undefined)`）。
#[test]
fn custom_panel_escape_cancels() {
    let Some(outcome) = run_custom_panel(
        r#"[{"waitStable":{"quietMs":2500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"\u001b"},{"delayMs":1500},{"mark":"esc"},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.log.contains("custom:result:undefined"),
        "Esc 之后组件应 `done(undefined)`。日志：\n{}",
        outcome.log
    );
}
