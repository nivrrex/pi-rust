//! `@file` 位置参数（ref `cli/file-processor.ts:25-90` + `main.ts:210-227`）。
//!
//! 语义：`@<path>` 把文件内容按 `<file name="绝对路径">\n内容\n</file>` 拼进初始消息；
//! **空文件跳过**；文件不存在 ⇒ `Error: File not found: <path>`、退出 1。
//!
//! 此前 `file_args` 被解析（两处 push）却**从未被消费** ⇒ `@x` 被当普通 prompt 文本
//! 发出去，模型看到的是字面量路径。

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pi-rs")
}

fn setup(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("agent")).expect("agent");
    std::fs::write(
        root.join("agent/models.json"),
        r#"{"providers":{"mock":{"apiKey":"k","models":[{"id":"m","provider":"mock","api":"openai-completions","baseUrl":"http://127.0.0.1:1/v1"}]}}}"#,
    )
    .expect("models");
    std::fs::write(
        root.join("agent/settings.json"),
        r#"{"defaultProvider":"mock","defaultModel":"m"}"#,
    )
    .expect("settings");
    std::fs::create_dir_all(root.join("work")).expect("work");
}

/// 文件不存在 ⇒ 明确报错并退出 1（ref `Console: File not found`）。
#[test]
fn missing_file_fails_with_a_clear_message() {
    let tmp = tempfile::tempdir().expect("tmp");
    setup(tmp.path());
    let result = Command::new(bin())
        .current_dir(tmp.path().join("work"))
        .env("PI_CODING_AGENT_DIR", tmp.path().join("agent"))
        .env("HOME", tmp.path())
        .args(["-p", "@/definitely/missing.txt"])
        .output()
        .expect("运行");
    assert!(!result.status.success(), "应退出非 0");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("File not found"),
        "应说明文件不存在：{stderr}"
    );
}

/// 文本内容必须按 `<file name="…">\n内容\n</file>` 拼进初始消息（用 mock 抓请求体校验）。
#[test]
fn file_content_reaches_the_request_body() {
    let tmp = tempfile::tempdir().expect("tmp");
    setup(tmp.path());
    let target = tmp.path().join("work/note.txt");
    std::fs::write(&target, "ATFILE-SENTINEL").expect("写文件");

    // 起 mock，记请求体
    let mock = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug/mock-provider");
    if !mock.is_file() {
        eprintln!("跳过：未构建 mock-provider");
        return;
    }
    let log = tmp.path().join("req.jsonl");
    let scenario =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/S01.json");
    let mut child = Command::new(&mock)
        .current_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .args([scenario.to_str().unwrap(), "--port", "0", "--request-log"])
        .arg(&log)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("起 mock");
    let port = {
        use std::io::BufRead;
        let stdout = child.stdout.take().expect("stdout");
        let mut line = String::new();
        std::io::BufReader::new(stdout)
            .read_line(&mut line)
            .expect("读端口");
        line.split(':')
            .next_back()
            .unwrap_or("0")
            .trim()
            .to_string()
    };
    // 让模型指向 mock
    std::fs::write(
        tmp.path().join("agent/models.json"),
        format!(
            r#"{{"providers":{{"mock":{{"apiKey":"k","models":[{{"id":"m","provider":"mock","api":"openai-completions","baseUrl":"http://127.0.0.1:{port}/v1"}}]}}}}}}"#
        ),
    )
    .expect("models");

    let _ = Command::new(bin())
        .current_dir(tmp.path().join("work"))
        .env("PI_CODING_AGENT_DIR", tmp.path().join("agent"))
        .env("HOME", tmp.path())
        .arg("--no-session")
        .arg("-p")
        .arg(format!("@{}", target.display()))
        .output()
        .expect("运行");
    let _ = child.kill();
    let _ = child.wait();

    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.contains("ATFILE-SENTINEL"),
        "文件内容应进请求体：{text:.300}"
    );
    assert!(text.contains("<file name="), "应带 file 包装：{text:.300}");
}
