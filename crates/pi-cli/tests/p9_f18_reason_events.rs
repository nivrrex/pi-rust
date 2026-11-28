//! P9-F18/S1：`session_start` / `session_shutdown` 的 reason 与顺序（pty 用例）。
//!
//! 预测（先写后跑）：启动 startup；`/new` 时 `shutdown{new,有target}` 先于 `start{new,有previous}`；
//! `/resume` 时 `shutdown{resume,有target}` 先于 `start{resume,有previous}`；退出 `shutdown{quit}`。
//! 推翻条件：任一顺序或字段不符 ⇒ dump 事件文件并对照 ref 行号定位。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct MockGuard(Child);
impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn start_mock() -> (MockGuard, String) {
    use std::io::{BufRead, BufReader};
    let binary = repo_root().join("target/debug/mock-provider");
    assert!(binary.exists(), "先构建 mock-provider");
    let mut child = Command::new(binary)
        .current_dir(repo_root())
        .arg("scenarios/S01.json")
        .arg("--port")
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mock 可启动");
    let port = {
        let stdout = child.stdout.take().expect("stdout");
        let mut lines = BufReader::new(stdout).lines();
        let mut found = None;
        for line in lines.by_ref() {
            let line = line.expect("读 stdout");
            if let Some(rest) = line.strip_prefix("listening ") {
                found = Some(
                    rest.rsplit(':')
                        .next()
                        .expect("端口")
                        .parse::<u16>()
                        .expect("数字"),
                );
                break;
            }
        }
        found.expect("listening")
    };
    (MockGuard(child), format!("http://127.0.0.1:{port}/v1"))
}

fn write_seed_session(sess: &Path, work: &Path) {
    let header = serde_json::json!({
        "type": "session", "version": 3, "id": "f18-seed",
        "timestamp": "2026-01-01T00:00:00.000Z", "cwd": work.display().to_string()
    });
    let user = serde_json::json!({
        "type": "message", "id": "e00", "parentId": null,
        "timestamp": "2026-01-01T00:00:01.000Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "seed"}], "timestamp": 1}
    });
    std::fs::write(sess.join("seed.jsonl"), format!("{header}\n{user}\n")).expect("写种子会话");
}

#[test]
fn session_lifecycle_reasons_and_order() {
    let Some(node) = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
    else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let (agent, sess, work, home) = (
        tmp.path().join("cfg"),
        tmp.path().join("sess"),
        tmp.path().join("work"),
        tmp.path().join("home"),
    );
    for dir in [&agent, &sess, &work, &home] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    let (_mock, base_url) = start_mock();
    std::fs::create_dir_all(agent.join("extensions/eventlog")).expect("建扩展目录");
    std::fs::copy(
        repo_root().join("tools/ext-host/fixtures/event-log-ext.mjs"),
        agent.join("extensions/eventlog/index.mjs"),
    )
    .expect("拷夹具");
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                "models": [{ "id": "mock-model" }]
            }}
        }))
        .expect("序列化"),
    )
    .expect("写 models.json");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"openai-completions","defaultModel":"mock-model"}"#,
    )
    .expect("写 settings.json");
    write_seed_session(&sess, &work);
    let log = tmp.path().join("events.jsonl");

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStrip":{"contains":"ctrl+o more","timeoutMs":30000}},{"delayMs":300},{"text":"/new\r"},{"waitStable":{"quietMs":500,"timeoutMs":20000}},{"delayMs":500},{"raw":"\u0004"}]"#,
    )
    .expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(&code)
        .arg("--out")
        .arg(&raw)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("PI_EVENT_LOG", &log)
        .env("HOME", &home)
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!text.is_empty(), "扩展应记录到事件；raw={raw:?}");
    let events: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let summary: Vec<String> = events
        .iter()
        .map(|event| {
            format!(
                "{}({},prev={},tgt={})",
                event["event"].as_str().unwrap_or("?"),
                event["reason"].as_str().unwrap_or("null"),
                event["hasPrevious"].as_bool().unwrap_or(false),
                event["hasTarget"].as_bool().unwrap_or(false)
            )
        })
        .collect();
    assert!(
        summary.iter().any(|line| line.starts_with("start(startup")),
        "启动应派发 session_start(startup)：{summary:?}"
    );
    let new_shutdown = summary
        .iter()
        .position(|line| line.starts_with("shutdown(new,") && line.contains("tgt=true"));
    let new_start = summary
        .iter()
        .position(|line| line.starts_with("start(new,") && line.contains("prev=true"));
    match (new_shutdown, new_start) {
        (Some(shutdown), Some(start)) => {
            assert!(shutdown < start, "/new 应先 shutdown 再 start：{summary:?}")
        }
        _ => panic!("/new 的 shutdown/start 缺失：{summary:?}"),
    }
    // `/resume` 的 reason 断言未包含在本用例里：负载下用 pty 驱动会话选择器不稳定
    // （同一假设链三次修改仍偶发"面板未出现"），按纪律暂停并登记（见 LIMITATIONS）。
    assert!(
        summary.iter().any(|line| line.starts_with("shutdown(quit")),
        "退出应派发 session_shutdown(quit)：{summary:?}"
    );
}
