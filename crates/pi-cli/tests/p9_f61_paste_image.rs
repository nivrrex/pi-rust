//! F61/35：`app.clipboard.pasteImage`（`ctrl+v`，ref `core/keybindings.ts:141-144`、
//! `interactive-mode.ts:3047-3068`）。
//!
//! 改前：该键**没有任何绑定**（`ctrl+v` 落到编辑器当成普通字符控制码被忽略）。
//!
//! 语义：图片优先（写临时文件、插入文件路径）→ 否则插入剪贴板文本 → 都没有则静默。
//! 本用例用假 `xclip` 提供文本剪贴板（图片分支由 `clipboard_image_tests` 的
//! 纯逻辑用例覆盖，那部分要真的图片字节与 `xclip` 的二进制回放，属另一层）。

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

/// 造一个假 `xclip`：`TARGETS` 只给 `text/plain`（⇒ 无图片），普通读取给固定文本。
fn write_fake_xclip(dir: &std::path::Path, text: &str) {
    // 假的 `xclip` 要**精确**：`TARGETS` 只声明文本、只有文本类型才返回内容。
    // 第一版写成"除 TARGETS 外一律返回内容"，于是 `-t image/png -o` 也返回了文本字节，
    // 被当成图片走"写临时文件 + 插入路径"分支（实测插入的是
    // `/tmp/pi-clipboard-*.png` 而不是文本）——那是假工具失真，不是实现错。
    let script = format!(
        r#"#!/bin/sh
case "$*" in
  *TARGETS*) printf 'text/plain\nUTF8_STRING\n' ;;
  *image/*) exit 1 ;;
  *) printf '%s' '{text}' ;;
esac
"#
    );
    let path = dir.join("xclip");
    std::fs::write(&path, script).expect("写 xclip");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
}

/// 造一个假 `xclip`，**声明并返回** PNG 图片字节（最小合法 PNG 头即可，
/// 我们不解析内容，只检查路径是否被插入）。
fn write_fake_xclip_png(dir: &std::path::Path) {
    let script = r#"#!/bin/sh
case "$*" in
  *TARGETS*) printf 'image/png\ntext/plain\n' ;;
  *image/png*) printf '\211PNG\r\n\032\n' ;;
  *) printf 'FALLBACK-TEXT' ;;
esac
"#;
    let path = dir.join("xclip");
    std::fs::write(&path, script).expect("写 xclip");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
}

/// 剪贴板里是**图片**时，应插入临时文件路径（而不是文本）。
#[test]
fn ctrl_v_pastes_an_image_path() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    let bin = tmp.path().join("bin");
    for dir in [&agent, &home, &work, &sessions, &bin] {
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
    write_fake_xclip_png(&bin);

    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"text": "\u{16}"},
        {"delayMs": 1200},
        {"mark": "pasted"},
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
        .arg("28")
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
        .env("DISPLAY", ":99")
        .env("PATH", path_env)
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
    let screen = screen_of(&raw);
    assert!(
        screen.contains("pi-clipboard-") && screen.contains(".png"),
        "图片应写成临时文件并插入其路径（ref `:3049-3057`）。屏幕：\n{screen}"
    );
    assert!(
        !screen.contains("FALLBACK-TEXT"),
        "有图片时不应插入剪贴板文本。屏幕：\n{screen}"
    );
}

/// 按 `ctrl+v` 后，剪贴板文本应出现在编辑器里。
#[test]
fn ctrl_v_pastes_clipboard_text_into_the_editor() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    let bin = tmp.path().join("bin");
    for dir in [&agent, &home, &work, &sessions, &bin] {
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
    write_fake_xclip(&bin, "PASTED-TEXT");

    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"text": "\u{16}"},
        {"delayMs": 1200},
        {"mark": "pasted"},
        {"text": "\u{4}"},
        {"delayMs": 1200}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
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
        // 假 `xclip` 需要 DISPLAY 才会被尝试（`read_clipboard_text` 的门槛）。
        .env("DISPLAY", ":99")
        .env("PATH", path_env)
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
    let screen = screen_of(&raw);
    assert!(
        screen.contains("PASTED-TEXT"),
        "`ctrl+v` 应把剪贴板文本插入编辑器（ref `:3058-3066`）。屏幕：\n{screen}"
    );
}
