//! P9-F11：`pi.appendEntry` 在三个入口都要落盘（F10 真机缺陷的回归）。
//!
//! - TUI：pty 里跑 TUI，扩展在 `session_start` 追加条目，退出后检查会话文件；
//! - rpc：`--mode rpc` 启动后直接检查会话文件（`session_start` 已派发）。
//!
//! 两者都依赖 `set_session_file` 在对应入口被调用：**去掉那行即失败**。

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
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/append-ext.mjs"))
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

fn custom_entries(sess: &Path) -> usize {
    let mut count = 0;
    if let Ok(dir) = std::fs::read_dir(sess) {
        for entry in dir.filter_map(Result::ok) {
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            count += text
                .lines()
                .filter(|line| line.contains("\"type\":\"custom\"") && line.contains("customType"))
                .count();
        }
    }
    count
}

fn node_path() -> Option<PathBuf> {
    Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
        .map(PathBuf::from)
}

#[test]
fn tui_entry_writes_append_entry() {
    let Some(node) = node_path() else {
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
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, r#"[{"delayMs":2500},{"raw":"\u0004"}]"#).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(&code)
        .arg("--out")
        .arg(&raw)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", tmp.path().join("home"))
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    assert!(
        custom_entries(&sess) >= 1,
        "TUI 入口应把 appendEntry 写进会话文件"
    );
}

/// rpc 入口**已知未覆盖**：会话文件是懒创建的（首次 `prompt` 才建），而 `session_start`
/// 在启动时就派发，此时 `set_session_file` 还没拿到路径 ⇒ appendEntry 落空。
/// 修法需要"先决定路径再建 writer"（ref 的 `SessionManager` 就持有路径），
/// 属于 rpc 会话生命周期改动，留作后续批次；本测试在未修前**预期失败**，用 `#[ignore]` 标记。
#[test]
fn rpc_entry_writes_append_entry() {
    let Some(node) = node_path() else {
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
    // rpc 模式：喂一个 `get_state` 让进程跑起来再退出。
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
        // `prompt` 才会建会话并派发 `session_start`（`get_state` 只是读状态）。
        writeln!(
            stdin,
            "{}",
            serde_json::json!({"id": 1, "method": "prompt", "params": {"message": "hi"}})
        )
        .expect("写请求");
        stdin.flush().expect("flush");
    }
    // 条件等待（不是固定延迟）：轮询会话文件里是否出现 custom 条目，最多 30s。
    // 全量并行时 rpc 的启动（node 宿主 + mock）可能远超 2.5s。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline && custom_entries(&sess) == 0 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        custom_entries(&sess) >= 1,
        "rpc 入口应把 appendEntry 写进会话文件"
    );
}
