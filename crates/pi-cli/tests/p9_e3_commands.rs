//! P9 E3：`registerCommand` 的分发（ref `agent-session.ts:1766-1789`）。
//!
//! `/name args` 命中扩展命令 ⇒ 调用 handler、**不**跑 agent；未命中 ⇒ 照常当 prompt。

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
    let target = agent.join("extensions/cmd");
    std::fs::create_dir_all(&target).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/command-ext.mjs"))
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
fn extension_commands_are_dispatched_before_the_prompt() {
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
    let log = tmp.path().join("events.log");

    let run = |prompt: &str| {
        Command::new(env!("CARGO_BIN_EXE_pi-rs"))
            .current_dir(&work)
            .arg("--print")
            .arg(prompt)
            .env("PI_CODING_AGENT_DIR", &agent)
            .env("PI_CODING_AGENT_SESSION_DIR", &sess)
            .env("PI_NODE", &node)
            .env("PI_EXT_TEST_LOG", &log)
            .env("HOME", tmp.path().join("home"))
            .output()
            .expect("pi-rs 可运行")
    };

    let handled = run("/record hello world");
    assert!(handled.status.success(), "{:?}", handled.status);
    assert!(handled.stdout.is_empty(), "命令不应产生正文输出");
    let events = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        events.contains("command:record:hello world"),
        "命令处理器应收到参数：{events}"
    );
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap_or_default()
            .lines()
            .count(),
        0,
        "命令不应触发 provider 请求"
    );

    // 未命中的 `/xxx` 仍按普通 prompt 发送。
    let not_handled = run("/unknown thing");
    assert!(not_handled.status.success(), "{:?}", not_handled.status);
    assert!(
        std::fs::read_to_string(&request_log)
            .unwrap_or_default()
            .lines()
            .count()
            > 0,
        "未命中命令应照常走 prompt"
    );

    // handler 抛错：按 ref 也视为"已处理"（退出码 0、不发请求）。
    let before = std::fs::read_to_string(&request_log)
        .unwrap_or_default()
        .lines()
        .count();
    let boom = run("/boom");
    assert!(boom.status.success(), "{:?}", boom.status);
    let after = std::fs::read_to_string(&request_log)
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(before, after, "抛错的命令不应发请求");
}
