//! P9-F29/S4d：`ctx.getContextUsage()` 在 `session_start` 时就必须可用
//! （ref `agent-session.ts:3858-3904`：`{tokens, contextWindow, percent}`）。
//!
//! 真机现象是底栏 `?/0 (auto)`：扩展读到的 usage 为 `undefined`，自己拼出 `?` 与窗口 0。

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

#[test]
fn context_usage_is_available_at_session_start() {
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
    let (_mock, base_url) = start_mock();
    let tmp = tempfile::tempdir().expect("临时目录");
    let (agent, sess, work) = (
        tmp.path().join("cfg"),
        tmp.path().join("sess"),
        tmp.path().join("work"),
    );
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                "models": [{ "id": "mock-model", "contextWindow": 200000 }]
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
    let ext = agent.join("extensions/usage");
    std::fs::create_dir_all(&ext).expect("建扩展目录");
    std::fs::copy(
        repo_root().join("tools/ext-host/fixtures/startup-usage-ext.mjs"),
        ext.join("index.mjs"),
    )
    .expect("拷扩展");

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStrip":{"contains":"ctrl+o more","timeoutMs":30000}},{"raw":"\u0004"}]"#,
    )
    .expect("写 keys");
    let log = tmp.path().join("ev.jsonl");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", Path::new(&node))
        .env("HOME", tmp.path().join("home"))
        .env("PI_EVENT_LOG", &log)
        .output()
        .expect("运行 pty-helper");
    assert!(output.status.success(), "pty-helper 失败");
    let text = std::fs::read_to_string(&log).expect("事件文件");
    let first: serde_json::Value =
        serde_json::from_str(text.lines().next().expect("至少一条事件")).expect("JSON");
    assert_eq!(
        first["window"].as_u64(),
        Some(200_000),
        "会话开始时就该有 contextWindow：{first}"
    );
    // F30/S1：TS 实测 `session_start` 为 `{tokens: 0, percent: 0}`（会话投影为空），
    // 关键是不能为 null（null ⇒ 扩展显示 `?/0`）。
    assert_eq!(
        first["percent"].as_f64(),
        Some(0.0),
        "启动时 percent 应为 0（非 null）：{first}"
    );
    assert_eq!(
        first["tokens"].as_u64(),
        Some(0),
        "启动时 tokens 应为 0：{first}"
    );
}
