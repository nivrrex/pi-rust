//! F42/Q1：扩展的 stdout/stderr 不得污染 TUI 画面。
//!
//! F41 把宿主 node 的 stdout/stderr 从 tty 改成管道，并把它们转发到 pi-rs 自身的 stderr；
//! 结果是扩展日志（`[remote-control] relay mode…` 之类）直接写进 TUI 画面，穿插在
//! `[Extensions]` 段与分隔线之间。本用例用 pty 跑真实二进制 + 合成扩展（load /
//! `session_start` / 延迟三处各写 stderr 与 stdout），断言画面里**看不到**这些行。

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
    std::fs::create_dir_all(agent.join("extensions/noisy")).expect("建扩展目录");
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
    std::fs::write(
        agent.join("extensions/noisy/package.json"),
        r#"{"type":"module","name":"noisy"}"#,
    )
    .expect("写扩展 package.json");
    std::fs::write(
        agent.join("extensions/noisy/index.js"),
        concat!(
            "export default function noisy(pi) {\n",
            "  process.stderr.write(\"[noisy] stderr-at-load\\n\");\n",
            "  console.log(\"[noisy] stdout-at-load\");\n",
            "  pi.on(\"session_start\", () => {\n",
            "    process.stderr.write(\"[noisy] stderr-at-session-start\\n\");\n",
            "    console.log(\"[noisy] stdout-at-session-start\");\n",
            "    setTimeout(() => {\n",
            "      process.stderr.write(\"[noisy] stderr-after-300ms\\n\");\n",
            "      console.log(\"[noisy] stdout-after-300ms\");\n",
            "    }, 300);\n",
            "  });\n",
            "}\n",
        ),
    )
    .expect("写扩展入口");
}

/// 在 pty 里跑 TUI，返回 (raw 字节, 扩展日志文件内容)。
fn run_tui() -> Option<(Vec<u8>, String)> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    prepare(&agent, &home, &work);
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"delayMs":2500},{"delayMs":1500},{"raw":"\u0004"},{"delayMs":800}]"#,
    )
    .expect("写 keys");
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
        .arg(&keys)
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
    let log = std::fs::read_to_string(agent.join("extensions.log")).unwrap_or_default();
    Some((raw, log))
}

/// TUI 画面的字节流里不得出现扩展日志（它们只允许出现在日志文件与退出时的 stderr）。
#[test]
fn extension_logs_do_not_pollute_the_tui_screen() {
    let Some((raw, log)) = run_tui() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    // 退出**之前**的帧里不应出现任何 `[noisy]` 行。
    // 退出时会一次性把日志打到 stderr（这也是 pty 的一部分），因此只检查到 `?2004l` 之前。
    let screen = match text.rfind("\u{1b}[?2004l") {
        Some(index) => &text[..index],
        None => text.as_str(),
    };
    assert!(
        !screen.contains("[noisy]"),
        "扩展日志污染了 TUI 画面：{:?}",
        screen
            .lines()
            .filter(|line| line.contains("[noisy]"))
            .take(3)
            .collect::<Vec<_>>()
    );
    // 文本不得丢失：6 条都必须进日志文件。
    for marker in [
        "stderr-at-load",
        "stdout-at-load",
        "stderr-at-session-start",
        "stdout-at-session-start",
        "stderr-after-300ms",
        "stdout-after-300ms",
    ] {
        assert!(
            log.contains(marker),
            "扩展日志丢了 `{marker}`：{log:?}"
        );
    }
}
