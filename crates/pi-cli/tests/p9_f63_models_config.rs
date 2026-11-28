//! F63/B：`models.json` 解析失败**不得致命退出**（ref `core/model-config.ts:279-315`）。
//!
//! ref 的 `ModelConfig.load` 对三类失败一律**返回空 Map + error 字符串**，调用方继续运行；
//! 错误由 `interactive-mode.ts:1144-1146` 以 `models.json error: …` 呈现。
//! pi-rs 此前在四个入口把它当致命错误 `ExitCode::from(1)` —— 用户手改
//! `models.json` 打错一个逗号，程序就再也起不来。
//!
//! 判据分两层：
//! 1. 纯函数 `load_models_config` 永不返回"失败"，只返回 `config + error`；
//! 2. 真进程：坏文件下进程**不是被解析错误打断**（stderr 出现 ref 文案而非解析 panic）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::Command;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 四种坏文件。
const BAD: &[(&str, &str)] = &[
    ("trailing_comma", r#"{"providers":{"p":{},}}"#),
    ("invalid_json", r#"{"providers": "#),
    ("wrong_types", r#"{"providers":"not-an-object"}"#),
    ("empty_file", ""),
];

/// 纯函数层：坏文件 ⇒ 空配置 + 错误文案，**不 panic、不返回 Err**。
#[test]
fn load_models_config_never_fails_on_bad_files() {
    for (name, body) in BAD {
        let tmp = tempfile::tempdir().expect("临时目录");
        std::fs::write(tmp.path().join("models.json"), body).expect("写");
        let loaded = pi_cli::settings::load_models_config(tmp.path());
        assert!(
            loaded.error.is_some(),
            "{name}: 坏文件应带回错误文案，实际 {loaded:?}"
        );
        let text = loaded.error.clone().unwrap_or_default();
        assert!(
            text.starts_with("Failed to parse models.json:")
                || text.starts_with("Failed to load models.json:")
                || text.starts_with("Invalid models.json schema:"),
            "{name}: 文案应对齐 ref `model-config.ts:287-312`，实际 {text:?}"
        );
    }
}

/// 文件不存在是**正常状态**（只放 `auth.json` 的机器），不带错误。
#[test]
fn missing_file_is_not_an_error() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let loaded = pi_cli::settings::load_models_config(tmp.path());
    assert!(loaded.error.is_none(), "缺文件不应报错：{loaded:?}");
}

/// 合法文件照常解析。
#[test]
fn valid_file_is_parsed() {
    let tmp = tempfile::tempdir().expect("临时目录");
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}"#,
    )
    .expect("写");
    let loaded = pi_cli::settings::load_models_config(tmp.path());
    assert!(
        loaded.error.is_none(),
        "合法文件不应报错：{:?}",
        loaded.error
    );
}

/// 真进程：坏 `models.json` 下 TUI **能进界面**（不是被解析错误打断）。
///
/// 与 TS 的对照：同一份坏文件 TS `rc=0`（回退内置模型），改前 pi-rs `rc=1`。
/// 本用例只断言"不因**解析**错误退出"这一层；回退到哪个模型属 C 节。
#[test]
fn bad_models_json_does_not_abort_the_tui() {
    let Some(node) = which("node") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(agent.join("models.json"), r#"{"providers":{"p":{},}}"#).expect("写 models");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"m1"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        serde_json::to_string(&serde_json::json!([
            {"waitStable": {"quietMs": 2500, "timeoutMs": 60000}},
            {"mark": "s"},
            {"text": "\u{4}"},
            {"delayMs": 1000}
        ]))
        .expect("序列化"),
    )
    .expect("写 keys");
    let code = tmp.path().join("code.txt");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
        .arg("--exit-code-file")
        .arg(&code)
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("COLORTERM", "")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("跑 pty");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("pi-rs: ") || !stderr.contains("解析失败"),
        "不应再把解析失败当成致命错误（stderr：{stderr}）"
    );
    let raw = std::fs::read_to_string(tmp.path().join("raw.bin")).unwrap_or_default();
    assert!(
        raw.contains("Failed to parse models.json"),
        "应按 ref `model-config.ts:295-303` 的文案告警，实际屏幕不含该文案"
    );
    let _ = Path::new("");
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
