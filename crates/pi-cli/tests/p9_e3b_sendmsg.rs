//! P9-1-E3b / S4：`pi.sendUserMessage` 排入的用户消息会在下一轮注入。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn node_path() -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

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

fn prepare(agent: &Path, base_url: &str) {
    let target = agent.join("extensions/sendmsg");
    std::fs::create_dir_all(&target).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/sendmsg-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
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

#[test]
fn send_user_message_is_injected_next_round() {
    let Some(node) = node_path() else {
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
    let request_log = tmp.path().join("requests.jsonl");
    let (_mock, base_url) = start_mock(&request_log);
    prepare(&agent, &base_url);

    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("hi")
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

    let requests = std::fs::read_to_string(&request_log).expect("读 requests.jsonl");
    let lines: Vec<&str> = requests
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 2, "应有两轮请求：{requests}");
    let second: serde_json::Value = serde_json::from_str(lines[1]).expect("JSON");
    let body: serde_json::Value =
        serde_json::from_str(second["body"].as_str().expect("body")).expect("JSON");
    let injected = body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .any(|m| m["role"] == "user" && m["content"].to_string().contains("SECOND-TURN"));
    assert!(injected, "第二轮请求应含扩展排入的用户消息：{body}");
}
