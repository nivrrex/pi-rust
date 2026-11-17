//! F61/37：`rpc` 退出时也必须派发 `session_shutdown` 并把扩展动作帧处理完。
//!
//! 同 F61/36 修好的 `--print`：三个入口（print / TUI / rpc）应当一致。
//! `--print` 与 TUI 已补，本用例钉住 rpc。
//!
//! 判据：扩展只在 `session_shutdown` 里 `appendEntry`，之后关闭 stdin 让进程退出，
//! 条目必须落到会话文件。

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
    let target = agent.join("extensions/shutdown-append");
    std::fs::create_dir_all(&target).expect("建目录");
    let body = std::fs::read_to_string(
        repo_root().join("tools/ext-host/fixtures/append-on-shutdown-ext.mjs"),
    )
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

fn custom_shutdown_entries(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| std::fs::read_to_string(entry.path()).unwrap_or_default())
                .filter(|text| text.contains("\"customType\":\"shutdown-note\""))
                .count()
        })
        .unwrap_or(0)
}

/// rpc：`prompt` 跑完 → 关 stdin 触发退出 → `session_shutdown` 的 appendEntry 必须落盘。
#[test]
fn rpc_mode_flushes_session_shutdown_append_entries() {
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
    let (_mock, base_url) = start_mock();
    prepare(&agent, &base_url);

    let mut child = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--mode")
        .arg("rpc")
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", tmp.path().join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("启动 rpc");
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("stdin");
        // 先跑一轮，确保会话文件已建（`prompt` 才会建会话）。
        writeln!(
            stdin,
            "{}",
            serde_json::json!({"id": 1, "method": "prompt", "params": {"message": "hi"}})
        )
        .expect("写请求");
        stdin.flush().expect("flush");
    }
    // 等会话文件出现（说明 prompt 已经建好并写盘）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
    while std::time::Instant::now() < deadline {
        let has_session = std::fs::read_dir(&sess)
            .map(|entries| entries.filter_map(Result::ok).count())
            .unwrap_or(0)
            > 0;
        if has_session {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // **关 stdin ⇒ 进程正常退出**（不是 kill）：只有正常退出路径才会走收尾。
    drop(child.stdin.take());
    let _ = child.wait();

    // 条件等待：退出收尾是异步的，给 10s。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline && custom_shutdown_entries(&sess) == 0 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        custom_shutdown_entries(&sess) >= 1,
        "rpc 退出应派发 session_shutdown 并落盘其 appendEntry。会话目录：{}",
        sess.display()
    );
}
