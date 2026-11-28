//! F63/C：`-c` / `--session` 恢复时，会话里的 provider 或模型已不存在**不得致命退出**
//! （ref `core/sdk.ts:199-228`）。
//!
//! ref 的顺序：先尝试用会话里的 `(provider, model)` 恢复（`:201-205`），
//! 失败则记 `modelFallbackMessage = "Could not restore model <p>/<m>"`（`:206-208`），
//! 再 `findInitialModel` 用 settings 的 default 重找（`:212-228`），
//! 实在没有才给 `No models available`（`:225`）——**任何一步都不退出**。
//!
//! 覆盖四种情形：provider 被删、模型被删、provider 在但无凭据、models.json 同时损坏。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Command, Stdio};

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| std::path::PathBuf::from(text))
}

/// 造一份会话：assistant 用指定的 `(provider, model)`。
fn write_session(path: &Path, provider: &str, model: &str, cwd: &Path) {
    let lines = [
        serde_json::json!({
            "type": "session", "version": 3,
            "id": "01a1a9a8-0000-7000-8000-0000000000c2",
            "timestamp": "2026-01-01T00:00:00.000Z",
            "cwd": cwd.to_string_lossy().to_string()
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
                        "api": "openai-completions", "provider": provider, "model": model,
                        "usage": {"input": 10, "output": 2, "cacheRead": 0, "cacheWrite": 0,
                                  "cost": {"input": 0.0, "output": 0.0, "cacheRead": 0.0,
                                           "cacheWrite": 0.0, "total": 0.0}},
                        "stopReason": "stop"}
        }),
    ];
    let text = lines
        .iter()
        .map(|line| serde_json::to_string(line).expect("序列化"))
        .collect::<Vec<String>>()
        .join("\n");
    std::fs::write(path, format!("{text}\n")).expect("写会话");
}

/// 跑 TUI（`--session`），返回 (退出码, 屏幕文本)。
fn run_session(
    models_json: &str,
    auth_json: &str,
    provider: &str,
    model: &str,
) -> Option<(i32, String)> {
    let node = which("node")?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(agent.join("models.json"), models_json).expect("写 models");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"m1"}"#,
    )
    .expect("写 settings");
    std::fs::write(agent.join("auth.json"), auth_json).expect("写 auth");
    let session = sess.join("s.jsonl");
    write_session(&session, provider, model, &work);

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        serde_json::to_string(&serde_json::json!([
            {"waitStable": {"quietMs": 2500, "timeoutMs": 60000}},
            {"text": "\u{4}"},
            {"delayMs": 1000}
        ]))
        .expect("序列化"),
    )
    .expect("写 keys");
    let code = tmp.path().join("code.txt");
    let raw = tmp.path().join("raw.bin");
    Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
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
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("COLORTERM", "")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("跑 pty");
    let exit = std::fs::read_to_string(&code)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
        .unwrap_or(-1);
    Some((exit, std::fs::read_to_string(&raw).unwrap_or_default()))
}

const GOOD_MODELS: &str = r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}"#;
const GOOD_AUTH: &str = r#"{"p":{"type":"api_key","key":"k"}}"#;

/// 情形 1：会话里的 **provider 已被删除**。
#[test]
fn missing_provider_does_not_abort() {
    let Some((exit, screen)) = run_session(GOOD_MODELS, GOOD_AUTH, "gone-provider", "gone-model")
    else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert_eq!(
        exit, 0,
        "provider 不存在不得致命退出（ref `sdk.ts:206-208` 只打提示）"
    );
    assert!(
        screen.contains("m1"),
        "应回退到 settings 的 defaultModel（ref `:212-228` 的 findInitialModel）"
    );
}

/// 情形 2：会话里的 **模型已被删除**（provider 还在）。
#[test]
fn missing_model_does_not_abort() {
    let Some((exit, screen)) = run_session(GOOD_MODELS, GOOD_AUTH, "p", "gone-model") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert_eq!(exit, 0, "模型不存在不得致命退出");
    assert!(screen.contains("m1"), "应回退到 settings 的 defaultModel");
}

/// 情形 3：provider 在 `models.json` 里但**没有凭据**。
#[test]
fn provider_without_credentials_does_not_abort() {
    let Some((exit, screen)) = run_session(GOOD_MODELS, "{}", "p", "m1") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert_eq!(
        exit, 0,
        "无凭据不得致命退出（ref 允许 model=undefined 进交互模式，由用户 /login 补齐）"
    );
    assert!(
        screen.contains("No models available") || screen.contains("m1"),
        "应给出 ref 的 `No models available` 提示或回退到可用模型"
    );
}

/// 情形 4：**`models.json` 同时损坏**（B 与 C 叠加）。
#[test]
fn broken_models_json_plus_missing_provider_does_not_abort() {
    let Some((exit, _screen)) = run_session(
        r#"{"providers":{"p":{},}}"#,
        GOOD_AUTH,
        "gone-provider",
        "gone-model",
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert_eq!(exit, 0, "坏 models.json + provider 缺失 不得致命退出");
}
