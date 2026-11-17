//! F61/42：项目不受信任时，TUI 必须在会话区显示警告
//! （ref `interactive-mode.ts:4070-4094` 的 `renderProjectTrustWarningIfNeeded`）。
//!
//! 改前：完全没有这条提示。用户在项目里放了 `.pi/settings.json` 或 `.pi/skills`，
//! 却看不到任何"它们被忽略"的说明——只能自己发现行为异常。
//!
//! 触发条件**两个都成立**才警告（ref `:4072-4074`）：不受信任 **且** 存在需要
//! 信任才能生效的项目资源。

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

/// 跑一次 TUI，返回屏幕文本。`project_resources` 为真时在工作目录造 `.pi/skills`
/// （触发"需要信任的项目资源"）。
fn run_tui(project_resources: bool, trusted: bool) -> Option<String> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sess");
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
    if trusted {
        std::fs::write(
            agent.join("trust.json"),
            format!("{{\"{}\": true}}\n", pi_core::trust::normalize_path(&work)),
        )
        .expect("写 trust.json");
    }
    if project_resources {
        // `.pi/skills` 是需要信任才能生效的项目资源（ref `trust-manager.ts:51-59`）。
        std::fs::create_dir_all(work.join(".pi/skills/demo")).expect("建目录");
        std::fs::write(
            work.join(".pi/skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: demo skill\n---\n\nbody\n",
        )
        .expect("写 SKILL.md");
    }

    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2500, "timeoutMs": 90000}},
        {"mark": "start"},
        {"text": "\u{4}"},
        {"delayMs": 1200}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("120")
        .arg("--rows")
        .arg("30")
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

/// 不受信任 + 有项目资源 ⇒ 显示警告。
#[test]
fn untrusted_project_with_resources_shows_the_warning() {
    let Some(screen) = run_tui(true, false) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("This project is not trusted"),
        "应显示 ref 的信任警告（interactive-mode.ts:4085）。屏幕：\n{screen}"
    );
    // 整句会被**折行**（120 列下 `then restart pi.` 落到下一行），因此分两段检查。
    assert!(
        screen.contains("Use /trust to save a trust decision"),
        "警告应告知如何修复。屏幕：\n{screen}"
    );
    assert!(
        screen.contains("restart pi."),
        "警告应说明需要重启。屏幕：\n{screen}"
    );
}

/// 已受信任 ⇒ 不显示（即使有项目资源）。
#[test]
fn trusted_project_shows_no_warning() {
    let Some(screen) = run_tui(true, true) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !screen.contains("This project is not trusted"),
        "已受信任不应警告。屏幕：\n{screen}"
    );
}

/// 不受信任但**没有**需要信任的资源 ⇒ 不显示（用户没有任何损失）。
#[test]
fn untrusted_project_without_resources_shows_no_warning() {
    let Some(screen) = run_tui(false, false) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !screen.contains("This project is not trusted"),
        "没有受控资源时不应警告。屏幕：\n{screen}"
    );
}
