//! F61/23：`keybindings.json` 的覆盖必须**真的改变按键行为**。
//!
//! 改前状态：键位硬编码在 `tui.rs` 的 `match` 里，`keybindings.json` 读都不读
//! ⇒ 用户自定义键位完全不生效（文件存在也没效果、无提示），`/hotkeys` 面板
//! 与 `/reload` 的 "Reloaded keybindings" 都是空话。
//!
//! 判据用 `PI_TEST_DUMP` 的 `step-N.json` → `footer.model`（状态层事实）：
//! 把 `app.model.cycleForward` 从默认的 `ctrl+p` 改成 `ctrl+n` 之后，
//! 按 `ctrl+n` 必须切换模型，而按 `ctrl+p` 不再切换。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
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

/// 跑一次，返回每步 `footer.model` 的序列。
fn run(keys: &str, keybindings: Option<&str>) -> Option<Vec<String>> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    let steps = tmp.path().join("steps");
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
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m1","name":"m1","contextWindow":200000,"maxTokens":1024},{"id":"m2","name":"m2","contextWindow":200000,"maxTokens":1024}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");
    if let Some(text) = keybindings {
        std::fs::write(agent.join("keybindings.json"), text).expect("写 keybindings");
    }

    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("120")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg({
            let keys_path = tmp.path().join("keys.json");
            std::fs::write(
                &keys_path,
                serde_json::to_string(&serde_json::json!([
                    {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}}
                ]))
                .expect("序列化"),
            )
            .expect("写 keys");
            keys_path
        })
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_TEST_KEYS", keys)
        .env("PI_TEST_DUMP", &steps)
        .env("PI_TEST_TAIL", "30")
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
    let mut out = Vec::new();
    for index in 0..32 {
        let path = steps.join(format!("step-{index}.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            break;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(model) = value
            .get("footer")
            .and_then(|footer| footer.get("model"))
            .and_then(serde_json::Value::as_str)
        {
            out.push(model.to_string());
        }
    }
    Some(out)
}

/// 基线：默认键位下 `ctrl+p` 切换模型。
///
/// 这条先钉住"默认行为没被键位层改坏"（`map_key_sequence_with` 在无覆盖时
/// 必须与旧映射逐字节相同）。
#[test]
fn default_ctrl_p_cycles_models() {
    let Some(models) = run("ctrl+p", None) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(models.len() >= 2, "应至少两步快照：{models:?}");
    assert_eq!(models[0], "m1", "初态应是 defaultModel");
    assert_eq!(
        models.last().map(String::as_str),
        Some("m2"),
        "默认键位下 ctrl+p 应切到 m2。序列：{models:?}"
    );
}

/// 把 `app.model.cycleForward` 改成 `ctrl+n` 之后：`ctrl+n` 切换、`ctrl+p` 不切换。
#[test]
fn keybindings_file_remaps_cycle_forward() {
    let Some(models) = run(
        "ctrl+p,ctrl+n",
        Some(r#"{"app.model.cycleForward":"ctrl+n"}"#),
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(models.len() >= 3, "应至少三步快照：{models:?}");
    assert_eq!(models[0], "m1", "初态应是 defaultModel");
    assert_eq!(
        models[1], "m1",
        "把 cycleForward 改到 ctrl+n 之后，ctrl+p 不应再切换模型。序列：{models:?}"
    );
    assert_eq!(
        models.last().map(String::as_str),
        Some("m2"),
        "ctrl+n 应切换模型（它是新的 cycleForward）。序列：{models:?}"
    );
}

/// 旧动作名（`cycleModelForward`）应被迁移（ref `KEYBINDING_NAME_MIGRATIONS`）。
#[test]
fn legacy_action_names_are_migrated() {
    let Some(models) = run("ctrl+n", Some(r#"{"cycleModelForward":"ctrl+n"}"#)) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert_eq!(
        models.last().map(String::as_str),
        Some("m2"),
        "旧名 cycleModelForward 应迁移成 app.model.cycleForward。序列：{models:?}"
    );
}

fn screen_of(raw: &std::path::Path) -> String {
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

/// 跑 `/hotkeys`，返回屏幕文本。
fn hotkeys_screen(keybindings: Option<&str>) -> Option<String> {
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
    if let Some(text) = keybindings {
        std::fs::write(agent.join("keybindings.json"), text).expect("写 keybindings");
    }
    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"text": "/hotkeys"},
        {"delayMs": 800},
        {"text": "\u{1b}"},
        {"delayMs": 300},
        {"text": "\r"},
        {"delayMs": 2500},
        {"mark": "panel"},
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
        .arg("40")
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("运行 pty-helper");
    assert!(
        output.status.success(),
        "pty-helper 应成功（stderr：{}）",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(screen_of(&raw))
}

/// `/hotkeys` 面板必须反映 `keybindings.json` 里的键位。
///
/// 改前：面板是编译期常量，改配置后仍显示 `Ctrl+P`。
#[test]
fn hotkeys_panel_reflects_the_keybindings_file() {
    let Some(screen) = hotkeys_screen(Some(r#"{"app.model.cycleForward":"ctrl+n"}"#)) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("Keyboard Shortcuts"),
        "应打开 /hotkeys 面板。屏幕：\n{screen}"
    );
    assert!(
        screen.contains("Ctrl+N"),
        "面板应显示改后的 cycleForward 键 Ctrl+N（改前是编译期常量）。屏幕：\n{screen}"
    );
}
