//! 会话里记录的模型已不存在时的恢复行为（ref `sdk.ts:200-224` 的实际生效路径）。
//! `model-resolver.ts` 的 `restoreModelFromSession` 在 0.87.1 没有调用方，不是规格来源。
//!
//! ref 的四级回落：恢复到的模型有凭据 → 用它；否则记一条
//! `Could not restore model p/m (<reason>).`，再回落到**当前模型** →
//! `defaultModelPerProvider` 表 → **第一个可用模型**，并记
//! `Using <p>/<m>.`；全都没有才让 `model` 为空。
//!
//! 此前 pi-rs 在这一步直接 `return Ok(None)`（不进 agent loop），
//! 于是"上次用的模型被删掉"的会话进去后**既没有回落提示、也不能用**。

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pi-rs")
}

/// 造一个会话：模型指向 `acme-gone/mystery-9`（任何目录里都不存在）。
fn seed_session(root: &std::path::Path) -> std::path::PathBuf {
    let dir = root.join("sess/--proj--");
    std::fs::create_dir_all(&dir).expect("分区");
    let path = dir.join("s.jsonl");
    let header = format!(
        r#"{{"type":"session","version":3,"id":"s","timestamp":"2026-11-25T00:00:00.000Z","cwd":"{}"}}"#,
        root.join("work").display()
    );
    let model_change = r#"{"type":"model_change","id":"m1","parentId":null,"timestamp":"2026-11-25T00:00:01.000Z","provider":"acme-gone","modelId":"mystery-9"}"#;
    let user = r#"{"type":"message","id":"u1","parentId":"m1","timestamp":"2026-11-25T00:00:02.000Z","message":{"role":"user","content":"hello"}}"#;
    std::fs::write(&path, format!("{header}\n{model_change}\n{user}\n")).expect("写会话");
    path
}

fn write_agent(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("agent")).expect("agent");
    std::fs::create_dir_all(root.join("work")).expect("work");
    // 一个可用的 provider（baseUrl 指向不可达端口，但**能解析**就够了）。
    std::fs::write(
        root.join("agent/models.json"),
        r#"{"providers":{"live":{"apiKey":"k","models":[{"id":"live-1","provider":"live","api":"openai-completions","baseUrl":"http://127.0.0.1:1/v1","contextWindow":1000}]}}}"#,
    )
    .expect("models");
}

/// 恢复会话时模型不存在：TUI 必须进得去，并显示 ref 的回落提示
/// （`sdk.ts:205-224`：`Could not restore model p/m` + `. Using p/m`，
/// 由 `interactive-mode.ts:1149-1150` 的 `showWarning` 呈现）。
#[test]
fn missing_session_model_falls_back_with_a_message() {
    let tmp = tempfile::tempdir().expect("tmp");
    write_agent(tmp.path());
    let session = seed_session(tmp.path());
    let screen = run_tui(tmp.path(), &session);
    assert!(
        screen.contains("Could not restore model acme-gone/mystery-9"),
        "应说明恢复失败：{screen:.600}"
    );
    assert!(
        screen.contains("Using live/live-1"),
        "应在同一条消息里说明回落到了哪个模型：{screen:.600}"
    );
}

/// 用 pty 起 TUI，跑完按键序列后返回屏幕文本。
fn run_tui(root: &std::path::Path, session: &std::path::Path) -> String {
    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 1500, "timeoutMs": 60000}},
        {"text": "\u{4}"},
        {"delayMs": 800}
    ]);
    let keys = root.join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = root.join("raw.bin");
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("python3")
        .arg(repo.join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(root.join("code.txt"))
        .arg("--out")
        .arg(&raw)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(bin())
        .current_dir(root.join("work"))
        .env("PI_CODING_AGENT_DIR", root.join("agent"))
        .env("HOME", root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .arg("--session")
        .arg(session)
        .output()
        .expect("运行 pty-helper");
    assert!(
        output.status.success(),
        "pty-helper 应成功：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    strip_ansi(&std::fs::read_to_string(&raw).unwrap_or_default())
}

/// 去掉 SGR / OSC，只留可读文本。
fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
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
                rest = &after[consumed..];
            }
            Some(']') => {
                // OSC：跳到 BEL 或 ST
                let end = after.find('\u{7}').map(|i| i + 1).unwrap_or_else(|| {
                    after.find("\u{1b}\\").map(|i| i + 2).unwrap_or(after.len())
                });
                rest = &after[end..];
            }
            _ => rest = &after[1..],
        }
    }
    out.push_str(rest);
    out
}
