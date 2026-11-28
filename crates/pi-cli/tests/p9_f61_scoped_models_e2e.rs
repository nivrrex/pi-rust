//! F61/17：`/scoped-models` 面板端到端（ref `interactive-mode.ts:3088-3092`
//! → `scoped-models-selector.ts:97-220`）。
//!
//! 纯逻辑已由 `t_f61_scoped_models` 覆盖（10 项）；本用例验证**接线**：
//! 1. 命令打开面板（`Model Configuration` 标题 + 列出模型）；
//! 2. 空格切换后 `ctrl+s` **真的写进** `settings.json` 的 `enabledModels`；
//! 3. 全启用状态保存时**删除**该键（而不是写"列出全部"的显式列表）。

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

/// 跑一次 `/scoped-models`，返回（屏幕、settings.json 内容）。
fn run(keys_after_open: &[&str]) -> Option<(String, String)> {
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
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m1"}"#,
    )
    .expect("写 settings");
    // 两个模型：便于"关掉一个"后仍有可用模型。
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m1","name":"m1","contextWindow":200000,"maxTokens":1024},{"id":"m2","name":"m2","contextWindow":200000,"maxTokens":1024}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");

    let mut steps = vec![
        serde_json::json!({"waitStable": {"quietMs": 2000, "timeoutMs": 90000}}),
        serde_json::json!({"mark": "s0"}),
        serde_json::json!({"text": "/scoped-models"}),
        serde_json::json!({"delayMs": 600}),
        serde_json::json!({"text": "\u{1b}"}),
        serde_json::json!({"delayMs": 300}),
        serde_json::json!({"text": "\r"}),
        serde_json::json!({"delayMs": 2500}),
        serde_json::json!({"mark": "panel"}),
    ];
    for key in keys_after_open {
        steps.push(serde_json::json!({"text": key}));
        steps.push(serde_json::json!({"delayMs": 900}));
    }
    steps.push(serde_json::json!({"mark": "after"}));
    steps.push(serde_json::json!({"text": "\u{1b}"}));
    steps.push(serde_json::json!({"delayMs": 300}));
    steps.push(serde_json::json!({"text": "\u{4}"}));
    steps.push(serde_json::json!({"delayMs": 1500}));

    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
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
    let exit = std::fs::read_to_string(&code).unwrap_or_default();
    assert_eq!(exit, "0", "TUI 必须正常退出，不能依赖 helper 超时强杀");
    Some((
        screen_of(&raw),
        std::fs::read_to_string(agent.join("settings.json")).unwrap_or_default(),
    ))
}

/// 面板能打开并列出模型。
#[test]
fn scoped_models_opens_the_panel() {
    let Some((screen, _settings)) = run(&[]) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !screen.contains("Command /scoped-models is not implemented yet"),
        "/scoped-models 不应走\"未实现\"分支。屏幕：\n{screen}"
    );
    assert!(
        screen.contains("Model Configuration"),
        "应显示面板标题（ref :132-133）。屏幕：\n{screen}"
    );
    // 版式对齐 TS 实测：每行是 `名称 [provider]`（ref `:262-266`），
    // 不是 `provider/id`。夹具里 name 与 id 相同，所以显示 `m1 [p]`。
    assert!(
        screen.contains("m1 [p]") && screen.contains("m2 [p]"),
        "面板应列出两个模型（名称 [provider]）。屏幕：\n{screen}"
    );
}

/// Enter 切换 + `ctrl+s` 真的写进 `enabledModels`。
///
/// 切换键是 **Enter**（ref `scoped-models-selector.ts:331` 的 `tui.select.confirm`）；
/// 空格属于搜索框（`:406-408`），不会改动启用集合。
#[test]
fn scoped_models_ctrl_s_persists_the_selection() {
    let Some((screen, settings)) = run(&["\r", "\u{13}"]) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("Model Configuration"),
        "面板应打开。屏幕：\n{screen}"
    );
    assert!(
        settings.contains("enabledModels"),
        "ctrl+s 应把选择写进 settings.json 的 enabledModels。settings：\n{settings}"
    );
    assert!(
        !settings.contains("\"p/m1\""),
        "被 Enter 关掉的第一个模型不应出现在 enabledModels 里。settings：\n{settings}"
    );
}

/// 全启用状态保存 ⇒ **删除**该键（不写"列出全部"的显式列表）。
#[test]
fn scoped_models_saving_all_enabled_removes_the_key() {
    // `ctrl+a` 全开 → `ctrl+s`
    let Some((screen, settings)) = run(&["\u{01}", "\u{13}"]) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("Model Configuration"),
        "面板应打开。屏幕：\n{screen}"
    );
    assert!(
        !settings.contains("enabledModels"),
        "全启用时保存应删除 enabledModels（否则新增模型不再自动进循环）。settings：\n{settings}"
    );
}
