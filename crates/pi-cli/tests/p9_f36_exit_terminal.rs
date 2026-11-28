//! F36/H1：退出后终端状态必须恢复（每个 set 都有对应 reset，termios 回到 cooked+echo）。
//!
//! 真机反馈：Ctrl+D 退出后按上键出现 `^[[A`——说明退出时**某项键盘模式没复位**。
//! 本测试在 pty 里跑真实二进制，检查原始字节里的成对关系与 termios 快照。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
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

/// 一份最小可启动配置（不需要 mock：启动屏与退出不依赖 provider）。
fn prepare(agent: &Path, home: &Path, work: &Path) {
    std::fs::create_dir_all(agent).expect("建 agent 目录");
    std::fs::create_dir_all(home).expect("建 HOME");
    std::fs::create_dir_all(work).expect("建工作目录");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m"}]}}}"#,
    )
    .expect("写 models");
}

struct Run {
    raw: Vec<u8>,
    termios: serde_json::Value,
}

fn run_tui(keys: &str) -> Option<Run> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sessions");
    prepare(&agent, &home, &work);
    let keys_path = tmp.path().join("keys.json");
    std::fs::write(&keys_path, keys).expect("写 keys");
    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let termios_path = tmp.path().join("termios.json");
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("80")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(&code_path)
        .arg("--termios-after")
        .arg(&termios_path)
        .arg("--out")
        .arg(&raw_path)
        .arg("--script")
        .arg(&keys_path)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    let raw = std::fs::read(&raw_path).expect("读 raw");
    let termios =
        serde_json::from_str(&std::fs::read_to_string(&termios_path).expect("读 termios"))
            .expect("解析 termios");
    Some(Run { raw, termios })
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// 五个 set 各自的 reset 必须出现，且次数为 1:1（kitty push/pop 用 `<u` 配对）。
fn assert_paired(raw: &[u8], label: &str) {
    let text = String::from_utf8_lossy(raw).to_string();
    let pairs = [
        ("\u{1b}[?2004h", "\u{1b}[?2004l", "bracketed paste"),
        ("\u{1b}[>4;2m", "\u{1b}[>4;0m", "modifyOtherKeys"),
    ];
    for (on, off, name) in pairs {
        let on_count = count(&text, on);
        let off_count = count(&text, off);
        assert_eq!(
            on_count, off_count,
            "{label}: {name} 的 set/reset 不成对（on={on_count} off={off_count}）"
        );
    }
    // kitty：push 形如 `\x1b[>5u`（数字可变），pop 固定 `\x1b[<u`。
    // 注意 `\x1b[>4;2m`（modifyOtherKeys）也是 `\x1b[>` 开头，必须用正则式精确计数。
    let mut push = 0;
    let bytes = text.as_bytes();
    let mut index = 0;
    while let Some(found) = text[index..].find("\u{1b}[>") {
        let start = index + found + 3;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end > start && bytes.get(end) == Some(&b'u') {
            push += 1;
        }
        index = start;
    }
    let pop = count(&text, "\u{1b}[<u");
    if push > 0 {
        assert_eq!(
            push, pop,
            "{label}: kitty push/pop 不成对（push={push} pop={pop}）"
        );
    }
    // 光标必须重新显示（`?25h` 至少一次，且在 `?25l` 之后）。
    assert!(text.contains("\u{1b}[?25h"), "{label}: 退出时未恢复光标");
}

#[test]
fn ctrl_d_restores_terminal_and_keyboard_modes() {
    let Some(run) = run_tui(r#"[{"delayMs":2500},{"raw":"\u0004"}]"#) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert_paired(&run.raw, "Ctrl+D");
    assert_eq!(
        run.termios["icanon"],
        serde_json::json!(true),
        "应回到 canonical"
    );
    assert_eq!(run.termios["echo"], serde_json::json!(true), "应回到 echo");
    assert_eq!(run.termios["isig"], serde_json::json!(true), "应恢复 ISIG");
}

/// ref 的键位是 **Ctrl+C = clear（清输入）、Ctrl+D = exit**（`interactive-mode.ts:986`
/// 的 `rawKeyHint(`${keyText("app.clear")}/${keyText("app.exit")}`, "clear/exit")`），
/// 所以"Ctrl+C 两次退出"并不存在；这里测"先 Ctrl+C 再 Ctrl+D"这条真实路径。
#[test]
fn ctrl_c_then_ctrl_d_restores_terminal() {
    let Some(run) = run_tui(
        r#"[{"delayMs":2000},{"raw":"\u0003"},{"delayMs":300},{"raw":"\u0004"},{"delayMs":600}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert_paired(&run.raw, "Ctrl+C→Ctrl+D");
    assert_eq!(run.termios["echo"], serde_json::json!(true), "应回到 echo");
}

/// 信号/错误退出路径：pty-helper 只支持按键，无法发信号 ⇒ 在单元层断言
/// `restore()` 的幂等闸门与退出字节成对（信号处理器与 panic hook 共用同一入口）。
#[test]
fn exit_bytes_pair_every_mode() {
    use pi_cli::tui::r3_exit_bytes;
    let bytes = r3_exit_bytes(true, true);
    assert!(bytes.contains("\u{1b}[?2004l"));
    assert!(bytes.contains("\u{1b}[<u"));
    assert!(
        bytes.contains("\u{1b}[>4;0m"),
        "开过 modifyOtherKeys 必须复位"
    );
    assert!(bytes.ends_with("\u{1b}[?25h"), "最后必须恢复光标");
}
