//! F39/M2d：退出后 tty 必须在 500 ms 内保持 shell 的 termios，且不得留下存活子进程。
//!
//! 用户证据：退出后直接按上键会被**字面回显** `^[[A`（说明按键到达时 tty 仍在 ECHO/ICANON），
//! 空回车一次后恢复正常。本用例在 pty 里跑真实二进制，退出后立刻把 tty 设为 raw 并观察 500 ms，
//! 同时确认没有 `runner.mjs` 残留。

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
    std::fs::create_dir_all(agent.join("extensions/probe")).expect("建扩展目录");
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
        agent.join("extensions/probe/package.json"),
        r#"{"type":"module","name":"probe"}"#,
    )
    .expect("写扩展 package.json");
    std::fs::write(
        agent.join("extensions/probe/index.js"),
        "export default function probe(pi) {}\n",
    )
    .expect("写扩展入口");
}

/// 退出后 500 ms 内：①没有存活 runner.mjs；②termios 不被改回 cooked。
#[test]
fn exit_leaves_no_runner_and_keeps_raw_after_shell_takes_over() {
    let Some(_node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    prepare(&agent, &home, &work);
    let out = Command::new("python3")
        .arg(repo_root().join("tools/difftest/exit-timeline.py"))
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("HOME", &home)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", tmp.path().join("sess"))
        .output()
        .expect("运行 timeline 夹具");
    assert!(out.status.success(), "夹具应成功");
    let text = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(text.trim()).expect("解析 JSON");
    // ① 退出后 500 ms 内不得有存活的 runner.mjs（真机 ps 的同一判据）。
    let alive = json["child_alive_samples"]
        .as_object()
        .map(|map| map.values().any(|item| item["alive_at_500ms"] == true))
        .unwrap_or(false);
    assert!(!alive, "退出后不应残留 runner.mjs：{json}");
    // ② shell 把 tty 设为 raw 之后，500 ms 内不得被改回 cooked（ECHO/ICANON 一直关着）。
    assert_eq!(
        json["flags_changed_after_raw"], false,
        "退出后 500 ms 内 termios 被改动：{json}"
    );
    // ③ 最后一次写 tty 到进程返回的间隔应接近 ref（≤120 ms；改前实测 161 ms）。
    let gap = json["exit_after_last_write_ms"].as_f64().unwrap_or(0.0);
    assert!(
        gap <= 120.0,
        "退出收尾耗时 {gap} ms 偏长（ref drainInput idle=50ms）"
    );
}
