//! P9-F7c/S2（互操作 ④）：`-c` 恢复含 `bashExecution` 的会话后，
//! 请求体里普通项按 `bashExecutionToText` 进入 user 消息、`!!` 项不出现，
//! 且会话文件原有行**逐字节不变**、新行追加在后。

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

fn start_mock(request_log: &Path) -> (MockGuard, String) {
    use std::io::{BufRead, BufReader};
    let binary = repo_root().join("target/debug/mock-provider");
    assert!(binary.exists(), "先构建 mock-provider");
    let mut child = Command::new(binary)
        .current_dir(repo_root())
        .arg("scenarios/S01.json")
        .arg("--port")
        .arg("0")
        .arg("--request-log")
        .arg(request_log)
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

fn write_config(agent: &Path, base_url: &str) {
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
}

/// 从 request-log 里取第一条请求体里的 `messages`。
fn first_request_messages(log: &Path) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(log).expect("读请求日志");
    let line = text.lines().next().expect("至少一条请求");
    let record: serde_json::Value = serde_json::from_str(line).expect("请求日志是 JSON");
    let body = record
        .get("body")
        .and_then(|b| b.as_str())
        .expect("body 文本");
    let body: serde_json::Value = serde_json::from_str(body).expect("body 是 JSON");
    body["messages"].as_array().cloned().unwrap_or_default()
}

#[test]
fn resumed_session_projects_bash_execution_into_the_request() {
    let Some(node) = std::env::var("PI_NODE").ok().or_else(|| {
        Command::new("sh")
            .arg("-c")
            .arg("command -v node")
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|text| !text.is_empty())
    }) else {
        eprintln!("跳过：本机没有 node");
        return;
    };

    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("cfg");
    let sess = tmp.path().join("sess");
    let work = tmp.path().join("work");
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    let log = tmp.path().join("requests.jsonl");
    let (_mock, base_url) = start_mock(&log);
    write_config(&agent, &base_url);

    // 把 TS 生成的样本（含 excludeFromContext 项）拷到隔离的会话目录。
    let fixture =
        std::fs::read_to_string(repo_root().join("fixtures/session/bash-execution.jsonl"))
            .expect("读不到 TS 样本");
    // 会话按 cwd 分区/匹配（`session-manager.ts:1790-1793`），所以副本的 header `cwd` 必须是
    // 本次运行的工作目录，否则 `-c` 找不到它。
    let mut lines: Vec<String> = fixture.lines().map(str::to_string).collect();
    let mut header: serde_json::Value =
        serde_json::from_str(&lines[0]).expect("fixture 首行应是 header");
    header["cwd"] = serde_json::json!(work.display().to_string());
    lines[0] = serde_json::to_string(&header).expect("序列化 header");
    let session_path = sess.join("resume.jsonl");
    std::fs::write(&session_path, lines.join("\n") + "\n").expect("写会话");
    let before_lines = lines.clone();

    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("-c")
        .arg("next question")
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

    let messages = first_request_messages(&log);
    let joined = serde_json::to_string(&messages).expect("序列化消息");
    assert!(
        joined.contains("Ran `echo hi`"),
        "普通 bashExecution 应以文本进入请求体：{joined}"
    );
    assert!(
        !joined.contains("echo secret"),
        "excludeFromContext 项不得进请求体：{joined}"
    );
    // 顺序：执行记录文本出现在最后的用户提问之前。
    let bash_at = joined.find("Ran `echo hi`").expect("有执行记录");
    let question_at = joined.rfind("next question").expect("有本轮提问");
    assert!(bash_at < question_at, "执行记录应在本轮提问之前");

    // 会话文件：原有行逐字节不变，新行追加在后。
    let after = std::fs::read_to_string(&session_path).expect("读会话");
    let after_lines: Vec<String> = after.lines().map(str::to_string).collect();
    assert_eq!(
        &after_lines[..before_lines.len()],
        &before_lines[..],
        "恢复后原有行必须逐字节不变"
    );
    assert!(
        after_lines.len() > before_lines.len(),
        "本轮对话应追加新行：{} -> {}",
        before_lines.len(),
        after_lines.len()
    );
}
