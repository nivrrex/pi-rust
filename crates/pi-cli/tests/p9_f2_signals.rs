//! F2 追加：SIGTERM / **SIGHUP** 也走优雅关停并恢复终端
//! （ref `interactive-mode.ts:4233-4252`：非 Windows 上 `signals = ["SIGTERM", "SIGHUP"]`）。
//!
//! 断言两件事：终端回到 cooked（`termios-after` 的 `icanon`/`echo` 为真）、
//! 输出里出现退出序列（`\x1b[?2004l` 与 `\x1b[?25h`）。

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_with_signal(signal: &str) -> (String, serde_json::Value) {
    let tmp = tempfile::tempdir().expect("tmp");
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, "[]").expect("写 keys");
    let out = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let termios = tmp.path().join("termios.json");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(&code)
        .arg("--out")
        .arg(&out)
        .arg("--script")
        .arg(&keys)
        .arg("--signal-after-ms")
        .arg("900")
        .arg("--signal")
        .arg(signal)
        .arg("--termios-after")
        .arg(&termios)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--no-session")
        .env("PI_CODING_AGENT_DIR", tmp.path())
        .env("PI_CODING_AGENT_SESSION_DIR", tmp.path().join("sess"))
        .output()
        .expect("运行 pty-helper");
    assert!(
        output.status.success(),
        "pty-helper 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let raw = String::from_utf8_lossy(&std::fs::read(&out).expect("读 raw")).to_string();
    let flags: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&termios).expect("读 termios"))
            .expect("termios json");
    (raw, flags)
}

#[test]
fn sigterm_and_sighup_restore_the_terminal() {
    for signal in ["TERM", "HUP"] {
        let (raw, flags) = run_with_signal(signal);
        assert_eq!(
            flags.get("icanon").and_then(serde_json::Value::as_bool),
            Some(true),
            "SIG{signal} 后应回到 canonical 模式：{flags}"
        );
        assert_eq!(
            flags.get("echo").and_then(serde_json::Value::as_bool),
            Some(true),
            "SIG{signal} 后应恢复 echo：{flags}"
        );
        assert!(
            raw.contains("\u{1b}[?2004l"),
            "SIG{signal} 后应关闭括号粘贴（退出序列）"
        );
        assert!(
            raw.contains("\u{1b}[?25h"),
            "SIG{signal} 后应显示光标（退出序列）"
        );
    }
}
