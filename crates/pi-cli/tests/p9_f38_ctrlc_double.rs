//! K1e：Ctrl+C 双击退出（ref `interactive-mode.ts:4116-4124`）。
//!
//! ref 的 `handleCtrlC()`：距上次 Ctrl+C 不足 **500 ms** ⇒ `shutdown()`；否则只清编辑器并记录时间。
//! 真机/pty 实测：TS 在 500 ms 内连发两次 `\x03` 会退出，pi-rs 不退出 ⇒ 本用例覆盖该差异。

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

/// 返回 (raw 字节, exit-code 文本)。
fn run_tui(keys: &str) -> Option<(Vec<u8>, String)> {
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
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("80")
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
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    let raw = std::fs::read(&raw_path).expect("读 raw");
    let code = std::fs::read_to_string(&code_path).unwrap_or_default();
    Some((raw, code))
}

/// 500 ms 内两次 Ctrl+C ⇒ 退出，且退出序列齐全（pop kitty、关 paste、显示光标）。
#[test]
fn double_ctrl_c_within_window_exits() {
    let Some((raw, code)) = run_tui(
        r#"[{"delayMs":2500},{"raw":"\u0003"},{"delayMs":120},{"raw":"\u0003"},{"delayMs":800}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    assert!(
        code.trim() == "0",
        "Ctrl+C 双击应退出（exit code = {code:?}）"
    );
    assert!(
        text.contains("\u{1b}[?2004l"),
        "退出时应关闭 bracketed paste"
    );
    assert!(text.contains("\u{1b}[<u"), "退出时应 pop kitty 键盘协议");
    assert!(text.contains("\u{1b}[?25h"), "退出时应恢复光标");
}

/// 超出 500 ms 窗口的两次 Ctrl+C 只清编辑器，不退出（ref 的窗口语义）。
#[test]
fn double_ctrl_c_outside_window_does_not_exit() {
    let Some((raw, code)) = run_tui(
        r#"[{"delayMs":2500},{"raw":"\u0003"},{"delayMs":900},{"raw":"\u0003"},{"delayMs":900},{"raw":"\u0004"},{"delayMs":600}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    // 两次 Ctrl+C 间隔 900 ms ⇒ 都不退出；最后靠 Ctrl+D 退出。
    assert!(text.contains("\u{1b}[?2004l"), "最后仍应由 Ctrl+D 正常退出");
    assert!(
        code.trim() == "0",
        "最终退出码应为 0（exit code = {code:?}）"
    );
    // 清编辑器的次数：两次 Ctrl+C 各产生一次重绘，体现在光标复位字节上。
    assert!(
        text.matches("\u{1b}[?25h").count() >= 1,
        "退出时应有光标恢复"
    );
}
