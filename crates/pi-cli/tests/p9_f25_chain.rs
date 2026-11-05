//! P9-F25/S3a：扩展 `appendEntry` 与会话写入器的**同一条 id 链**。
//!
//! 用 `PI_TEST_DELAY_FIRST_FLUSH_MS` 让扩展的 `appendEntry` 先于 writer 首次落盘发生，
//! 然后检查会话文件的条目链：id 唯一、`parentId` 逐条指向上一项、无分叉
//! （ref `session-manager.ts:1288-1299` 的 `appendCustomEntry` 用 `parentId: this.leafId`）。

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

fn prepare(agent: &Path, base_url: &str) {
    let target = agent.join("extensions/append");
    std::fs::create_dir_all(&target).expect("建目录");
    std::fs::copy(
        repo_root().join("tools/ext-host/fixtures/append-ext.mjs"),
        target.join("index.mjs"),
    )
    .expect("拷扩展");
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
fn custom_entry_joins_the_same_id_chain() {
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
    let (_mock, base_url) = start_mock();
    prepare(&agent, &base_url);

    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("hi")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", tmp.path().join("home"))
        .env("PI_TEST_DELAY_FIRST_FLUSH_MS", "500")
        .output()
        .expect("pi-rs 可运行");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // 取最新会话文件，逐行解析成条目（跳过 header）。
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut session_path: Option<std::path::PathBuf> = None;
    for file in std::fs::read_dir(&sess)
        .expect("读会话目录")
        .filter_map(Result::ok)
    {
        session_path = Some(file.path());
        let text = std::fs::read_to_string(file.path()).unwrap_or_default();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                if value.get("type").and_then(|t| t.as_str()) != Some("session") {
                    entries.push(value);
                }
            }
        }
    }
    assert!(entries.len() >= 3, "应至少有条目：{entries:?}");
    let custom = entries
        .iter()
        .find(|entry| entry.get("type").and_then(|t| t.as_str()) == Some("custom"))
        .expect("应有 custom 条目");
    assert!(custom.get("id").is_some(), "custom 条目要有 id：{custom}");
    assert!(
        custom.get("parentId").is_some(),
        "custom 条目要有 parentId：{custom}"
    );

    // id 唯一、parentId 逐条指向前一条（允许 null 开头），且不分叉。
    let mut seen = std::collections::HashSet::new();
    let mut previous: Option<String> = None;
    for entry in &entries {
        let id = entry
            .get("id")
            .and_then(|v| v.as_str())
            .expect("条目应有 id")
            .to_string();
        assert!(seen.insert(id.clone()), "id 重复：{id}");
        let parent = entry
            .get("parentId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        assert_eq!(
            parent, previous,
            "parentId 应指向上一条（链式、无分叉）：{entry}"
        );
        previous = Some(id);
    }

    // S3b：用 **ref 真实 `SessionManager.open`** 加载同一文件，确认它认可这条链
    // （entries 数一致、branch 与 entries 同长即无分叉、leafId 是最后一条）。
    let ts_pkg_ok = std::env::var("TS_PI_ENTRY")
        .map(|entry| {
            std::path::Path::new(&entry)
                .ancestors()
                .nth(3)
                .map(|p| p.is_dir())
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if ts_pkg_ok {
        let path = session_path.expect("应有会话文件");
        let output = Command::new(node.clone())
            .arg(repo_root().join("tools/difftest/ts-oracle/verify-chain.mjs"))
            .arg(&path)
            .env(
                "TS_PI_ENTRY",
                std::env::var("TS_PI_ENTRY").unwrap_or_default(),
            )
            .output()
            .expect("运行 verify-chain");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("脚本应输出 JSON");
        assert_eq!(
            report["entries"].as_u64(),
            Some(entries.len() as u64),
            "ref 读到的条目数应与我们一致：{report}"
        );
        assert_eq!(
            report["branch"].as_u64(),
            Some(entries.len() as u64),
            "ref 的 branch 应覆盖全部条目（无分叉）：{report}"
        );
        assert_eq!(
            report["leafId"].as_str(),
            previous.as_deref(),
            "ref 的叶子应是我们写的最后一条：{report}"
        );
    }
}
