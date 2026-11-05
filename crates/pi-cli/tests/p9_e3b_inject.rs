//! P9-1-E3b / S3：`before_agent_start` 的返回值生效
//! （ref `agent-session.ts:1703`、`runner.ts:1312-1365`：`message` 注入 + `systemPrompt` 覆盖）。

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

#[test]
fn before_agent_start_injects_message_and_overrides_system_prompt() {
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
    let target = agent.join("extensions/inject");
    std::fs::create_dir_all(&target).expect("建目录");
    std::fs::write(
        target.join("index.mjs"),
        std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/inject-ext.mjs"))
            .expect("读 fixture"),
    )
    .expect("写扩展");
    let request_log = tmp.path().join("requests.jsonl");
    let (_mock, base_url) = start_mock(&request_log);
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

    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("hello")
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
    let first: serde_json::Value =
        serde_json::from_str(requests.lines().next().expect("有请求")).expect("JSON");
    let body: serde_json::Value =
        serde_json::from_str(first["body"].as_str().expect("body")).expect("JSON");
    let messages = body["messages"].as_array().expect("messages");

    // 1) 系统提示被覆盖。
    let system = messages
        .iter()
        .find(|m| m["role"] == "system")
        .map(|m| m["content"].to_string())
        .unwrap_or_default();
    assert!(
        system.contains("CUSTOM SYSTEM PROMPT"),
        "systemPrompt 覆盖应生效：{system}"
    );

    // 2) 注入的消息在**用户消息之前**，且内容到达模型（provider 边界折成 user 角色）。
    let injected = messages
        .iter()
        .position(|m| m["content"].to_string().contains("INJECTED-NOTE"))
        .expect("注入的消息应进请求体");
    let user = messages
        .iter()
        .position(|m| m["content"].to_string().contains("hello"))
        .expect("用户消息应在请求体里");
    assert!(injected < user, "注入的消息应排在用户消息之前");
}
