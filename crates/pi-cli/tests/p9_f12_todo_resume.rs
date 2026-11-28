//! P9-F12：todo 状态随 `-c` 恢复（`pi-handoff-gate` 用 toolResult.details.state 持久化，
//! 由 `session_start` 从 `getBranch()` 重建）。
//!
//! 断言：会话里已有 todo 的 toolResult 后，`-c` 续写时**请求体**里仍带着那次工具结果
//! （`details.state`），说明上下文重建成立。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct MockGuard(Child);
impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_mock(log: &Path) -> (MockGuard, String) {
    use std::io::{BufRead, BufReader};
    let binary = repo_root().join("target/debug/mock-provider");
    assert!(binary.exists(), "先构建 mock-provider");
    let mut child = Command::new(binary)
        .current_dir(repo_root())
        .arg("scenarios/S01.json")
        .arg("--port")
        .arg("0")
        .arg("--request-log")
        .arg(log)
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

#[test]
fn resumed_request_carries_previous_todo_state() {
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
    let (agent, sess, work) = (
        tmp.path().join("cfg"),
        tmp.path().join("sess"),
        tmp.path().join("work"),
    );
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    let log = tmp.path().join("requests.jsonl");
    let (_mock, base_url) = start_mock(&log);
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

    // 造一个"已含 todo 结果"的会话：header + 一条 todo 的 toolResult（details.state）。
    let session = sess.join("resume.jsonl");
    let header = serde_json::json!({
        "type": "session", "version": 3, "id": "f12-resume",
        "timestamp": "2026-01-01T00:00:00.000Z", "cwd": work.display().to_string()
    });
    let reminder = serde_json::json!({
        "type": "message", "id": "e00", "parentId": null, "timestamp": "2026-01-01T00:00:01.000Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "set todos"}], "timestamp": 1}
    });
    let tool_call = serde_json::json!({
        "type": "message", "id": "e01", "parentId": "e00", "timestamp": "2026-01-01T00:00:02.000Z",
        "message": {"role": "assistant", "content": [{"type": "toolCall", "id": "call_todo",
            "name": "todo", "arguments": {"action": "set", "items": ["hello", "ports"]}}],
            "api": "openai-completions", "provider": "openai-completions", "model": "mock-model",
            "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "reasoning": 0,
                "totalTokens": 2, "cost": {"input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0}},
            "stopReason": "toolUse", "rawStopReason": "tool_calls", "timestamp": 2}
    });
    let tool_result = serde_json::json!({
        "type": "message", "id": "e02", "parentId": "e01", "timestamp": "2026-01-01T00:00:03.000Z",
        "message": {"role": "toolResult", "toolCallId": "call_todo", "toolName": "todo",
            "content": [{"type": "text", "text": "[~] #1 hello\n[ ] #2 ports"}],
            "isError": false, "timestamp": 3,
            "details": {"action": "set", "state": {"items": [
                {"id": 1, "status": "in_progress", "text": "hello"},
                {"id": 2, "status": "pending", "text": "ports"}]}}}
    });
    std::fs::write(
        &session,
        format!("{header}\n{reminder}\n{tool_call}\n{tool_result}\n"),
    )
    .expect("写会话");

    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("-c")
        .arg("next")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("pi-rs 可运行");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = std::fs::read_to_string(&log).expect("读请求日志");
    let first = text.lines().next().expect("至少一条请求");
    let record: serde_json::Value = serde_json::from_str(first).expect("日志是 JSON");
    let body: serde_json::Value =
        serde_json::from_str(record["body"].as_str().expect("body")).expect("body 是 JSON");
    let messages = serde_json::to_string(&body["messages"]).expect("序列化 messages");
    assert!(
        messages.contains("hello") && messages.contains("ports"),
        "续写请求体应包含上一轮 todo 的内容：{messages}"
    );
}
