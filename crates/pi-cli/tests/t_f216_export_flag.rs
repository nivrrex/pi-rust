//! `--export <session.jsonl> [output]`（ref `main.ts:624-635` 的 `parsed.export`）。
//!
//! 语义：读**任意**会话文件 → 导出 → 打印 `Exported to: <path>` → 退出 0。
//! 此前它被塞进 `unsupported`，传了就报错，而 `--help` 里列着。

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pi-rs")
}

fn session(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("s.jsonl");
    let body = concat!(
        r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}"#,
        "\n",
        r#"{"type":"message","id":"m1","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"导出我"}]}}"#,
        "\n"
    );
    std::fs::write(&path, body).expect("写会话");
    path
}

#[test]
fn export_writes_html_and_reports_the_path() {
    let tmp = tempfile::tempdir().expect("tmp");
    let src = session(tmp.path());
    let out = tmp.path().join("out.html");
    let result = Command::new(bin())
        .arg("--export")
        .arg(&src)
        .arg(&out)
        .output()
        .expect("应能执行");
    assert!(result.status.success(), "应退出 0：{:?}", result);
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("Exported to:"), "应打印导出路径：{stdout}");
    let html = std::fs::read_to_string(&out).expect("应写出文件");
    assert!(html.starts_with("<!DOCTYPE html>"), "应是 HTML");
    assert!(html.contains("导出我"), "应含消息正文");
}

/// 不给输出路径时用默认名 `pi-session-<basename>.html`（ref 同规则）。
#[test]
fn export_uses_a_default_name_when_output_is_omitted() {
    let tmp = tempfile::tempdir().expect("tmp");
    let src = session(tmp.path());
    let result = Command::new(bin())
        .current_dir(tmp.path())
        .arg("--export")
        .arg(&src)
        .output()
        .expect("应能执行");
    assert!(result.status.success(), "应退出 0：{:?}", result);
    let produced = tmp.path().join("pi-session-s.html");
    assert!(produced.is_file(), "默认名应是 pi-session-s.html");
}

/// 源文件不存在 ⇒ 报错、退出 1（ref 抛 `File not found`）。
#[test]
fn missing_input_fails() {
    let tmp = tempfile::tempdir().expect("tmp");
    let result = Command::new(bin())
        .arg("--export")
        .arg(tmp.path().join("nope.jsonl"))
        .output()
        .expect("应能执行");
    assert!(!result.status.success(), "应退出非 0");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("No such file")
            || stderr.contains("not found")
            || stderr.contains("不存在"),
        "应说明文件不存在：{stderr}"
    );
}
