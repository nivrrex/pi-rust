//! P9-F22/S1b：`appendEntry` 与 SessionWriter **首次落盘**的竞态（确定性复现）。
//!
//! 用 `PI_TEST_DELAY_FIRST_FLUSH_MS` 把 writer 的首次整文件写入推迟，使 sink 线程的裸追加
//! 必然先落到同一路径；此时若首次落盘是"截断式整文件写入"且不并回已有行，custom 条目就会丢。
//!
//! 修复前：本测试**必然失败**；修复后：通过与 `p9_e3b_append` 一致。

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
fn first_flush_must_not_truncate_an_earlier_append_entry() {
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
        // 关键：把首次落盘推迟，让 sink 的追加必然先发生。
        .env("PI_TEST_DELAY_FIRST_FLUSH_MS", "500")
        .output()
        .expect("pi-rs 可运行");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut found = 0;
    for entry in std::fs::read_dir(&sess)
        .expect("读会话目录")
        .filter_map(Result::ok)
    {
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        found += text
            .lines()
            .filter(|line| line.contains("\"type\":\"custom\"") && line.contains("customType"))
            .count();
    }
    assert!(
        found >= 1,
        "首次落盘不得截断先前由 sink 追加的 custom 条目（found={found}）"
    );
}
