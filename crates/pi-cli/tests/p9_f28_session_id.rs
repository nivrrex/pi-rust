//! P9-F28/S1：`ctx.sessionManager.getSessionId()` 必须与会话文件 header 的 `id` 一致，
//! 且**启动后不发消息**时不落盘会话文件（ref `session-manager.ts` 的延迟落盘）。

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

fn prepare(base_url: &str, root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let (agent, sess, work) = (root.join("cfg"), root.join("sess"), root.join("work"));
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
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
    let dir = agent.join("extensions/sid");
    std::fs::create_dir_all(&dir).expect("建扩展目录");
    std::fs::copy(
        repo_root().join("tools/ext-host/fixtures/sessionid-ext.mjs"),
        dir.join("index.mjs"),
    )
    .expect("拷扩展");
    (agent, sess, work)
}

fn events(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("事件行是 JSON"))
        .collect()
}

fn header_id(sess: &Path) -> Option<String> {
    for file in std::fs::read_dir(sess)
        .expect("读会话目录")
        .filter_map(Result::ok)
    {
        let text = std::fs::read_to_string(file.path()).unwrap_or_default();
        if let Some(first) = text.lines().next() {
            let value: serde_json::Value = serde_json::from_str(first).ok()?;
            return value.get("id")?.as_str().map(str::to_string);
        }
    }
    None
}

fn session_files(sess: &Path) -> usize {
    std::fs::read_dir(sess)
        .expect("读会话目录")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .count()
}

#[test]
fn get_session_id_matches_session_header_and_stays_empty_at_startup() {
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
    let node = Path::new(&node);
    let (_mock, base_url) = start_mock();

    // A) print 模式发一条消息：事件里的 id 必须等于会话文件 header 的 id。
    let tmp = tempfile::tempdir().expect("临时目录");
    let (agent, sess, work) = prepare(&base_url, tmp.path());
    let log = tmp.path().join("ev.jsonl");
    let status = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("-p")
        .arg("hi")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", node)
        .env("HOME", tmp.path().join("home"))
        .env("PI_EVENT_LOG", &log)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .expect("运行 pi-rs");
    assert!(status.success(), "print 运行失败");
    let ids: Vec<String> = events(&log)
        .iter()
        .map(|event| event["id"].as_str().unwrap_or("<null>").to_string())
        .collect();
    let expected = header_id(&sess).expect("会话文件有 header id");
    assert!(
        ids.iter().any(|id| id == &expected),
        "session_start 里的 getSessionId 应等于 header id：事件={ids:?} header={expected}"
    );

    // B) TUI 不发消息直接退出：不新增会话文件，但 id 仍然可读（未落盘时来自宿主）。
    let tmp = tempfile::tempdir().expect("临时目录");
    let (agent, sess, work) = prepare(&base_url, tmp.path());
    let log = tmp.path().join("ev.jsonl");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStrip":{"contains":"ctrl+o more","timeoutMs":30000}},{"raw":"\u0004"}]"#,
    )
    .expect("写 keys");
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
        .env("PI_NODE", node)
        .env("HOME", tmp.path().join("home"))
        .env("PI_EVENT_LOG", &log)
        .output()
        .expect("运行 pty-helper");
    assert!(output.status.success(), "pty-helper 失败");
    assert_eq!(session_files(&sess), 0, "启动后不发消息不应创建会话文件");
    let ids: Vec<String> = events(&log)
        .iter()
        .map(|event| event["id"].as_str().unwrap_or("<null>").to_string())
        .collect();
    assert!(
        ids.iter().any(|id| id != "<null>" && !id.is_empty()),
        "TUI 启动时也应拿到会话 id：事件={ids:?}"
    );
}
