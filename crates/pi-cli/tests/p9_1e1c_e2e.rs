//! P9-1-E1c / S2a：mock provider 两轮端到端——模型调用扩展工具 → 宿主执行 →
//! `tool_result` 钩子脱敏 → 结果回填 → 下一轮请求体与落盘会话都是**替换后**的内容。
//!
//! 只读临时目录；`PI_NODE` 可覆盖 node 路径。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

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

struct MockGuard(Child);

impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// 启动 mock，返回 (守卫, base url, 请求日志路径)。
fn start_mock(scenario: &Path, log: &Path) -> (MockGuard, String) {
    let binary = repo_root().join("target/debug/mock-provider");
    assert!(
        binary.exists(),
        "先构建 mock-provider：cargo build -p mock-provider"
    );
    let mut child = Command::new(binary)
        .current_dir(repo_root())
        .arg(scenario)
        .arg("--port")
        .arg("0")
        .arg("--request-log")
        .arg(log)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mock-provider 可启动");
    let port = {
        use std::io::BufRead;
        let stdout = child.stdout.take().expect("已取走 stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
        let mut found = None;
        for line in lines.by_ref() {
            let line = line.expect("读 stdout");
            if let Some(rest) = line.strip_prefix("listening ") {
                let port = rest
                    .rsplit(':')
                    .next()
                    .expect("端口")
                    .parse::<u16>()
                    .expect("端口数字");
                found = Some(port);
                break;
            }
        }
        found.expect("mock 应打印 listening")
    };
    (MockGuard(child), format!("http://127.0.0.1:{port}/v1"))
}

fn write_config(agent: &Path, base_url: &str) {
    let cfg = serde_json::json!({
        "providers": {
            "openai-completions": {
                "baseUrl": base_url,
                "api": "openai-completions",
                "apiKey": "test-key",
                "models": [ { "id": "mock-model" } ]
            }
        }
    });
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&cfg).expect("序列化 models.json"),
    )
    .expect("写 models.json");
    let settings = serde_json::json!({
        "defaultProvider": "openai-completions",
        "defaultModel": "mock-model"
    });
    std::fs::write(
        agent.join("settings.json"),
        serde_json::to_string_pretty(&settings).expect("序列化 settings.json"),
    )
    .expect("写 settings.json");
    for (dir, source) in [("echo", "echo-ext.mjs"), ("hook", "hook-ext.mjs")] {
        let target = agent.join("extensions").join(dir);
        std::fs::create_dir_all(&target).expect("建扩展目录");
        let body =
            std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures").join(source))
                .expect("读 fixture");
        std::fs::write(target.join("index.mjs"), body).expect("写扩展");
    }
}

#[test]
fn two_round_extension_tool_roundtrip_is_redacted() {
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
    // 场景必须留在 `<repo>/scenarios/` 下：mock 把**场景文件的祖父目录**当仓库根来解析
    // `fixtures/sse/...`（`tools/mock-provider/src/main.rs:504-510`），放临时目录会读不到夹具。
    let scenario = repo_root().join("scenarios/S21-ext-e2e.json");
    let (_mock, base_url) = start_mock(&scenario, &request_log);
    write_config(&agent, &base_url);

    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("call ext_echo")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("pi-rs 可运行");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "print 应成功：status={:?}\nstdout={stdout}\nstderr={stderr}",
        output.status
    );

    // 1) 落盘会话：toolResult 是钩子替换后的内容，原文不落盘。
    let session_text = std::fs::read_dir(&sess)
        .expect("读会话目录")
        .filter_map(Result::ok)
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!session_text.is_empty(), "应写出会话文件");
    // 只看 toolResult 条目：模型自己的 toolCall 参数里本来就有 `SECRET-data`
    // （ref 的 `tool_result` 钩子只替换工具结果，不改模型的入参）。
    let mut tool_results = 0usize;
    for line in session_text
        .lines()
        .filter(|l| l.contains("\"toolResult\""))
    {
        let entry: serde_json::Value = serde_json::from_str(line).expect("会话行是 JSON");
        let text = entry["message"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        tool_results += 1;
        assert!(
            text.contains("[redacted]") && !text.contains("SECRET"),
            "toolResult 应是钩子替换后的内容：{text}"
        );
    }
    assert_eq!(tool_results, 1, "应恰好有一条 toolResult：{session_text}");

    // 2) 第二轮请求体：模型看到的是替换后的内容。
    let requests = std::fs::read_to_string(&request_log).expect("读 requests.jsonl");
    let lines: Vec<&str> = requests.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(lines.len() >= 2, "应有两轮请求：{requests}");
    let second: serde_json::Value = serde_json::from_str(lines[1]).expect("请求行是 JSON");
    let body: serde_json::Value =
        serde_json::from_str(second["body"].as_str().expect("body 是字符串"))
            .expect("body 是 JSON");
    let tool_messages: Vec<&serde_json::Value> = body["messages"]
        .as_array()
        .expect("messages 是数组")
        .iter()
        .filter(|m| m["role"] == "tool")
        .collect();
    assert_eq!(tool_messages.len(), 1, "第二轮应带一条工具消息：{body}");
    let content = tool_messages[0]["content"].to_string();
    assert!(
        content.contains("[redacted]") && !content.contains("SECRET"),
        "第二轮请求体里的工具结果是替换后的内容：{content}"
    );
}
