//! `--session-id <id>`（ref `main.ts:314-336` 的 `validateSessionIdFlags` + `createSessionManager`）。
//!
//! 契约：
//! 1. **用该 ID 新建会话**（文件头 `id` 就是它，文件名也用它）；
//! 2. 与 `--session` / `-c` / `--resume` **互斥**，同时给报错退出；
//! 3. ID 格式不合法时报错（ref `assertValidSessionId`）。
//!
//! 此前该 flag 被解析、但 `main` **从未使用** ⇒ 仍生成随机 UUID，flag 形同虚设。

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pi-rs")
}

fn setup(tmp: &std::path::Path) {
    std::fs::create_dir_all(tmp.join("agent")).expect("agent");
    std::fs::write(
        tmp.join("agent/models.json"),
        r#"{"providers":{"mock":{"apiKey":"k","models":[{"id":"m","provider":"mock","api":"openai-completions","baseUrl":"http://127.0.0.1:1/v1"}]}}}"#,
    )
    .expect("models");
    std::fs::write(
        tmp.join("agent/settings.json"),
        r#"{"defaultProvider":"mock","defaultModel":"m"}"#,
    )
    .expect("settings");
    std::fs::create_dir_all(tmp.join("work")).expect("work");
    std::fs::create_dir_all(tmp.join("sess")).expect("sess");
}

#[test]
fn session_id_is_used_for_the_new_session() {
    let tmp = tempfile::tempdir().expect("tmp");
    setup(tmp.path());
    let _ = Command::new(bin())
        .current_dir(tmp.path().join("work"))
        .env("PI_CODING_AGENT_DIR", tmp.path().join("agent"))
        .env("PI_CODING_AGENT_SESSION_DIR", tmp.path().join("sess"))
        .env("HOME", tmp.path())
        .args(["--session-id", "my-fixed-id", "-p", "x"])
        .output()
        .expect("运行");

    let files: Vec<std::path::PathBuf> = std::fs::read_dir(tmp.path().join("sess"))
        .expect("读会话目录")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(files.len(), 1, "应生成一个会话文件：{files:?}");
    let name = files[0]
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string();
    assert!(name.contains("my-fixed-id"), "文件名应含给定 ID：{name}");
    let text = std::fs::read_to_string(&files[0]).expect("读会话");
    let header: serde_json::Value =
        serde_json::from_str(text.lines().next().expect("header")).expect("解析");
    assert_eq!(
        header["id"], "my-fixed-id",
        "文件头 id 应是给定值：{header}"
    );
}

/// 与 `-c` 同时给 ⇒ 报错退出（ref `validateSessionIdFlags`）。
#[test]
fn session_id_conflicts_with_continue() {
    let tmp = tempfile::tempdir().expect("tmp");
    setup(tmp.path());
    let result = Command::new(bin())
        .current_dir(tmp.path().join("work"))
        .env("PI_CODING_AGENT_DIR", tmp.path().join("agent"))
        .env("PI_CODING_AGENT_SESSION_DIR", tmp.path().join("sess"))
        .env("HOME", tmp.path())
        .args(["--session-id", "x1", "-c", "-p", "hi"])
        .output()
        .expect("运行");
    assert!(!result.status.success(), "应退出非 0");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("--session-id") && stderr.contains("--continue"),
        "应说明二者互斥：{stderr}"
    );
}
