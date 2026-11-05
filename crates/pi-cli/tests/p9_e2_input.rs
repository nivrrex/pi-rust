//! P9 E2：`input` 钩子（ref `agent-session.ts:1553-1572`、`runner.ts:1412-1460`）。
//!
//! 三件事：`continue` 原样、`transform` 改写、`handled` 直接接管（不跑 agent）。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use pi_cli::ext_tools::InputOutcome;

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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn prepare_agent_dir(root: &Path) {
    let target = root.join("extensions/input");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/input-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

struct MockGuard(Child);

impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_mock(scenario: &str, log: Option<&Path>) -> (MockGuard, String) {
    use std::io::{BufRead, BufReader};
    let binary = repo_root().join("target/debug/mock-provider");
    assert!(
        binary.exists(),
        "先构建 mock-provider：cargo build -p mock-provider"
    );
    let mut command = Command::new(binary);
    command
        .current_dir(repo_root())
        .arg(format!("scenarios/{scenario}.json"))
        .arg("--port")
        .arg("0");
    if let Some(log) = log {
        command.arg("--request-log").arg(log);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mock-provider 可启动");
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
                        .expect("端口数字"),
                );
                break;
            }
        }
        found.expect("mock 应打印 listening")
    };
    (MockGuard(child), format!("http://127.0.0.1:{port}/v1"))
}

/// 启动宿主（`default_tools` 的副作用），返回日志路径。全局 `OnceLock`：每进程只能一次。
fn start_host(agent: &Path, log: &Path) -> Vec<std::sync::Arc<dyn pi_core::tools::tool::Tool>> {
    prepare_agent_dir(agent);
    std::env::set_var("PI_CODING_AGENT_DIR", agent);
    std::env::set_var("PI_NODE", node_path().expect("需要 node"));
    std::env::set_var("PI_EXT_TEST_LOG", log);
    pi_cli::ext_tools::configure_ctx(agent, "print", false);
    pi_cli::print_mode::default_tools(&pi_cli::args::Args::default())
}

#[test]
fn input_hooks_transform_handle_and_reach_provider() {
    if node_path().is_none() {
        eprintln!("跳过：本机没有 node");
        return;
    }
    // 宿主是进程级单例（`OnceLock`），两个测试并行跑会共用同一个 `PI_EXT_TEST_LOG`
    // ⇒ 日志串台。合成一条测试，顺序执行两段断言。
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("cfg");
    let sess = tmp.path().join("sess");
    let work = tmp.path().join("work");
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    let log = tmp.path().join("events.log");
    let _tools = start_host(&agent, &log);

    assert_eq!(
        pi_cli::ext_tools::run_input_hooks("plain text", "interactive"),
        InputOutcome::Continue
    );
    assert_eq!(
        pi_cli::ext_tools::run_input_hooks("!upper hello", "interactive"),
        InputOutcome::Transform("HELLO".to_string())
    );
    assert_eq!(
        pi_cli::ext_tools::run_input_hooks("!drop please", "rpc"),
        InputOutcome::Handled
    );
    let events = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(events.contains("input:interactive:plain text"), "{events}");
    assert!(events.contains("input:rpc:!drop please"), "{events}");

    // —— 端到端：`!upper hi` 经过钩子后，**发给模型的内容**是大写。——
    let request_log = tmp.path().join("requests.jsonl");
    let (_mock, base_url) = start_mock("S01", Some(&request_log));
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url,
                "api": "openai-completions",
                "apiKey": "k",
                "models": [{ "id": "mock-model" }],
            }}
        }))
        .expect("序列化"),
    )
    .expect("写 models.json");
    std::fs::write(
        agent.join("settings.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "defaultProvider": "openai-completions",
            "defaultModel": "mock-model"
        }))
        .expect("序列化"),
    )
    .expect("写 settings.json");

    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("!upper hi")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", node_path().expect("需要 node"))
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("pi-rs 可运行");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(output.status.success(), "print 应成功：{stderr}");
    let events = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        events.contains("input:interactive:!upper hi"),
        "钩子应收到原始输入：{events}"
    );
    // 发给模型的是**大写**后的文本，原始 `!upper hi` 不进请求体。
    let requests = std::fs::read_to_string(&request_log).expect("读 requests.jsonl");
    let first: serde_json::Value =
        serde_json::from_str(requests.lines().next().expect("有请求")).expect("请求行是 JSON");
    let body: serde_json::Value =
        serde_json::from_str(first["body"].as_str().expect("body 是字符串")).expect("body 是 JSON");
    // `before_agent_start` 的注入消息在 provider 边界也折成 `role:"user"`，
    // 因此要按内容挑出**真正的那条**用户消息。
    let user_message = body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|m| m["role"] == "user" && m["content"].to_string().contains("HI"))
        .expect("有 user 消息");
    // 文本可能直接是字符串，也可能是 `[{type:"text",text:…}]` 的块数组。
    let user_text = match &user_message["content"] {
        serde_json::Value::String(text) => text.clone(),
        other => other
            .as_array()
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|b| b["type"] == "text")
                    .and_then(|b| b["text"].as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default(),
    };
    assert_eq!(user_text, "HI", "模型应收到大写文本");
    assert!(
        !requests.contains("!upper hi"),
        "原始输入不应进请求体：{requests}"
    );

    // —— `handled`：不跑 agent，因此**没有新的 provider 请求**，退出码 0。——
    let before = requests.lines().count();
    let dropped = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("!drop please")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", node_path().expect("需要 node"))
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("pi-rs 可运行");
    assert!(
        dropped.status.success(),
        "handled 应正常退出：{}",
        String::from_utf8_lossy(&dropped.stderr)
    );
    let after = std::fs::read_to_string(&request_log)
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(after, before, "handled 之后不应再发请求");
    assert!(
        dropped.stdout.is_empty(),
        "handled 不应输出正文：{:?}",
        String::from_utf8_lossy(&dropped.stdout)
    );
}
