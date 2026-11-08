//! F43/R0：退出时只输出**本次会话新增**的扩展日志，不是文件全量。
//!
//! F42 让 TUI 模式的扩展日志落到 `<agentDir>/extensions.log` 并在退出时一次性输出；
//! 但输出的是整个文件 ⇒ 连跑两次会话时，第二次退出会把第一次的日志再打一遍
//! （真机：5 次启动后第 6 次退出输出了前 5 次全部内容）。

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

fn prepare(agent: &Path, home: &Path) {
    std::fs::create_dir_all(agent.join("extensions/noisy")).expect("建扩展目录");
    std::fs::create_dir_all(home).expect("建 HOME");
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
    // 每次启动打一条**唯一**标记（进程内单调计数不够：两次会话是两个进程）。
    std::fs::write(
        agent.join("extensions/noisy/index.js"),
        concat!(
            "export default function noisy(pi) {\n",
            "  process.stderr.write(`[noisy] boot ${process.pid}-${Date.now()}\\n`);\n",
            "}\n",
        ),
    )
    .expect("写扩展入口");
}

/// 统计文本里 `[noisy] boot …` 的行。
fn noisy_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            // 退出输出里第一条日志前面还挂着复位字节（`?2004l<u?25h`），
            // 只取 `[noisy] boot …` 起的部分，避免把转义序列算进行号内容。
            line.find("[noisy] boot ")
                .map(|index| line[index..].trim().to_string())
        })
        .collect()
}

#[test]
fn exit_prints_only_this_session_log_lines() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    prepare(&agent, &home);
    let out = Command::new("python3")
        .arg(repo_root().join("tools/difftest/tui-two-sessions.py"))
        .arg(&agent)
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .output()
        .expect("运行两次会话夹具");
    assert!(out.status.success(), "夹具应成功");
    let json: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).expect("解析 JSON");
    let first = json["first_exit_stderr"].as_str().unwrap_or_default();
    let second = json["second_exit_stderr"].as_str().unwrap_or_default();
    let log = json["log_after_second"].as_str().unwrap_or_default();

    let first_lines = noisy_lines(first);
    let second_lines = noisy_lines(second);
    assert_eq!(
        first_lines.len(),
        1,
        "第一次退出应输出 1 行：{first_lines:?}"
    );
    assert_eq!(
        second_lines.len(),
        1,
        "第二次退出只应输出**本次**新增的 1 行，实际 {} 行：{second_lines:?}",
        second_lines.len()
    );
    assert_ne!(
        first_lines[0], second_lines[0],
        "第二次输出不应是第一次那行"
    );
    // 文件仍累积保留两次会话的日志。
    let log_lines = noisy_lines(log);
    assert_eq!(log_lines.len(), 2, "日志文件应保留两行：{log_lines:?}");
    assert!(log_lines.contains(&first_lines[0]));
    assert!(log_lines.contains(&second_lines[0]));
}
