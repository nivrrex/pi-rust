//! F61/25：`/changelog`（ref `interactive-mode.ts:6505-6524` + `utils/changelog.ts`）。
//!
//! 语义：读**随包** `<packageDir>/CHANGELOG.md`，把全部条目按旧→新拼接（相对链接
//! 改写成固定 tag 的 GitHub URL），标题 `What's New`；没有条目时显示
//! `No changelog entries found.`（ref `:6515`）——**不是报错**。

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

/// 跑 `/changelog`，返回屏幕文本。`package_dir` 为 `None` 时不设 `PI_PACKAGE_DIR`。
fn run(package_dir: Option<&std::path::Path>) -> Option<String> {
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

    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"text": "/changelog"},
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
    let mut command = Command::new("python3");
    command
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
        .env("LC_ALL", "zh_CN.UTF-8");
    match package_dir {
        Some(dir) => {
            command.env("PI_PACKAGE_DIR", dir);
        }
        None => {
            command.env_remove("PI_PACKAGE_DIR");
        }
    }
    let output = command
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

/// 有 CHANGELOG 时：显示 `What's New` 与条目内容（不是"未实现"）。
#[test]
fn changelog_shows_entries_from_the_package() {
    // **不硬编码本机路径**（`scripts/precommit.sh` 的"仓库约定"检查会拒绝
    // 用户 home 下的绝对路径进仓库）。用临时目录造一份最小 CHANGELOG，
    // 这样用例自包含、任何机器都能跑。
    let tmp = tempfile::tempdir().expect("临时目录");
    std::fs::write(
        tmp.path().join("CHANGELOG.md"),
        "# Changelog\n\n## [0.87.1] - 2026-09-22\n\n### New Features\n\n- Something new\n\n## [0.87.0] - 2026-09-01\n\n- Older thing\n",
    )
    .expect("写 CHANGELOG");
    let Some(screen) = run(Some(tmp.path())) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !screen.contains("Command /changelog is not implemented yet"),
        "/changelog 不应再走未实现分支。屏幕：\n{screen}"
    );
    assert!(
        screen.contains("What's New"),
        "应显示 ref 的标题（interactive-mode.ts:6519）。屏幕：\n{screen}"
    );
    assert!(
        screen.contains("0.87.1") && screen.contains("Something new"),
        "应渲染出 changelog 内容。屏幕：\n{screen}"
    );
}

/// 没有 CHANGELOG 时：按 ref 显示 `No changelog entries found.`（不是报错、不是未实现）。
#[test]
fn missing_changelog_reports_no_entries() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let Some(screen) = run(Some(tmp.path())) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("No changelog entries found."),
        "缺文件时应显示 ref 的提示（interactive-mode.ts:6515）。屏幕：\n{screen}"
    );
    assert!(
        screen.contains("What's New"),
        "仍应有标题。屏幕：\n{screen}"
    );
}
