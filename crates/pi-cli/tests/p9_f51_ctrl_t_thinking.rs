//! F51/Z6：`ctrl+t` 切换思考块可见性。
//!
//! ref `core/keybindings.ts:119-121`：`app.thinking.toggle` 默认 **`ctrl+t`**；
//! `interactive-mode.ts:4429-4434`：
//! `hideThinkingBlock = !hideThinkingBlock` → `setHideThinkingBlock()`（写设置）
//! → `updateThinkingBlockVisibility()`（就地重建，不清屏）→
//! `showStatus("Thinking blocks: hidden|visible")`。
//!
//! 改前 pi-rs 的 `ctrl+t` 在主界面「无绑定」：不切换、无状态行、不写设置。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
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

fn prepare(agent: &std::path::Path, home: &std::path::Path, work: &std::path::Path) {
    for dir in [agent, home, work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    // `hideThinkingBlock: false` ⇒ 启动时思考块**可见**，ctrl+t 后应变 hidden。
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m","hideThinkingBlock":false}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m","contextWindow":197000}]}}}"#,
    )
    .expect("写 models");
    let session = concat!(
        "{\"type\":\"session\",\"version\":3,\"id\":\"s-ctrlt\",\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"cwd\":\"<CWD>\"}\n",
        "{\"type\":\"model_change\",\"id\":\"e1\",\"parentId\":null,\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"provider\":\"p\",\"modelId\":\"m\"}\n",
        "{\"type\":\"message\",\"id\":\"a1\",\"parentId\":\"e1\",\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"message\":{\"role\":\"assistant\",\"api\":\"openai-completions\",\"provider\":\"p\",\"model\":\"m\",\"content\":[{\"type\":\"thinking\",\"thinking\":\"SYNTH-THINKING-BLOCK\"},{\"type\":\"text\",\"text\":\"正文\"}],\"stopReason\":\"stop\",\"usage\":{\"input\":1,\"output\":1,\"cacheRead\":0,\"cacheWrite\":0,\"totalTokens\":2,\"cost\":{\"input\":0,\"output\":0,\"cacheRead\":0,\"cacheWrite\":0,\"total\":0}},\"timestamp\":2}}\n"
    )
    .replace("<CWD>", &work.to_string_lossy().replace('\\', "\\\\"));
    let sess = agent.join("sessions");
    std::fs::create_dir_all(&sess).expect("建会话目录");
    std::fs::write(sess.join("s.jsonl"), session).expect("写会话");
}

fn run_tui(keys: &str) -> Option<(Vec<u8>, String)> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    prepare(&agent, &home, &work);
    let keys_path = tmp.path().join("keys.json");
    std::fs::write(&keys_path, keys).expect("写 keys");
    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let status = Command::new("python3")
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
        .arg("--session")
        .arg(agent.join("sessions/s.jsonl"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", agent.join("sessions"))
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    let raw = std::fs::read(&raw_path).expect("读 raw");
    let settings = std::fs::read_to_string(agent.join("settings.json")).unwrap_or_default();
    Some((raw, settings))
}

/// `ctrl+t` 之后：状态行出现 `Thinking blocks: hidden`，且 `settings.json` 被写成 `true`。
#[test]
fn ctrl_t_toggles_thinking_visibility() {
    let Some((raw, settings)) = run_tui(
        r#"[{"delayMs":3000},{"raw":"\u0014"},{"delayMs":1200},{"raw":"\u0004"},{"delayMs":600}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    assert!(
        text.contains("Thinking blocks: hidden"),
        "ctrl+t 之后应出现状态行 `Thinking blocks: hidden`"
    );
    assert!(
        text.contains("SYNTH-THINKING-BLOCK") || text.contains("+ Thought"),
        "启动时应能看到思考块（或扩展标签）"
    );
    assert!(
        settings.contains("\"hideThinkingBlock\": true"),
        "ctrl+t 应把 hideThinkingBlock 写回 settings.json（ref `:4431`），实际：{settings}"
    );
}
