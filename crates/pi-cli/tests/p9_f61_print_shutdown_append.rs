//! F61/36：`--print` 退出前必须派发 `session_shutdown` 并把扩展动作帧处理完。
//!
//! 背景：LIMITATIONS 里记过 `p9_e3b_append` 在 8 路并行下"整条 `custom` 条目丢失"，
//! 当时的假设是"退出时宿主动作帧没排空"。我用 8/16 路并行压了 240+ 次**没复现**，
//! 但它指出了一条**确定性**的同类缺口：TUI 退出路径
//! （`tui.rs`：`emit_session_shutdown_sync("quit")` → `shutdown_host`）做对了，
//! **`--print` 路径两条都没做** ⇒ 只在 `session_shutdown` 里 `appendEntry` 的扩展，
//! 在 print 模式下条目**必然丢失**。这不需要偶发条件，单次即可复现。

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

/// `--print` 退出时，`session_shutdown` 里的 `appendEntry` 必须落到会话文件。
#[test]
fn print_mode_flushes_session_shutdown_append_entries() {
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

    let session_text = std::fs::read_dir(&sess)
        .expect("读会话目录")
        .filter_map(Result::ok)
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        session_text.contains("\"customType\":\"shutdown-note\""),
        "`--print` 退出应派发 session_shutdown 并落盘其 appendEntry。会话文件：\n{session_text}"
    );
}
