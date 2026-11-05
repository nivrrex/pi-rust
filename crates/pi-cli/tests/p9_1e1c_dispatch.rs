//! P9-1-E1c / S2b + S2c：扩展事件在 **TUI** 与 **rpc** 两条路径上的派发。
//!
//! - rpc：脚本化客户端跑一轮，断言 stdout 出现 `agent_end`，且测试扩展记到
//!   `session_start:startup` → `agent_end:<n>`；
//! - TUI：mock provider 驱动 tmux（`paste-buffer -p` 输入，不注入转义序列），
//!   断言同样的顺序；再用 `/resume` 断言 `session_start:resume`。
//!
//! 只读临时目录；`PI_NODE` 可覆盖 node 路径；没有 tmux 时 TUI 部分显式跳过。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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

fn start_mock(scenario: &str) -> (MockGuard, String) {
    let binary = repo_root().join("target/debug/mock-provider");
    assert!(
        binary.exists(),
        "先构建 mock-provider：cargo build -p mock-provider"
    );
    let mut child = Command::new(binary)
        .current_dir(repo_root())
        .arg(format!("scenarios/{scenario}.json"))
        .arg("--port")
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mock-provider 可启动");
    let port = {
        let stdout = child.stdout.take().expect("已取走 stdout");
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
    std::fs::write(
        agent.join("settings.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "defaultProvider": "openai-completions",
            "defaultModel": "mock-model"
        }))
        .expect("序列化 settings.json"),
    )
    .expect("写 settings.json");
    let target = agent.join("extensions/events");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    // `reason-ext` 不退订，`/resume` 的第二次 session_start 才收得到。
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/reason-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

fn read_log(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn wait_for_log(path: &Path, needle: &str, limit: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if read_log(path).iter().any(|line| line.contains(needle)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// S2c：rpc 一轮 → `agent_end` 派发给扩展（并在 stdout 上出现）。
#[test]
fn rpc_dispatches_agent_end() {
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
    let (_mock, base_url) = start_mock("S01");
    write_config(&agent, &base_url);
    let log = tmp.path().join("events.log");

    let mut child = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--mode")
        .arg("rpc")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("PI_EXT_TEST_LOG", &log)
        .env("HOME", tmp.path().join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("pi-rs 可启动");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, r#"{{"type":"prompt","message":"hi","id":"1"}}"#).expect("写命令");
        stdin.flush().expect("flush");
    }
    let stdout = child.stdout.take().expect("stdout");
    let reader = BufReader::new(stdout);
    let mut saw_agent_end = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.contains("\"agent_end\"") {
            saw_agent_end = true;
            break;
        }
        if Instant::now() > deadline {
            break;
        }
    }
    // 扩展侧是**另一个进程**：stdout 上的 `agent_end` 先于宿主把事件投递给扩展，
    // 立即 kill 会让扩展来不及写日志（并行跑全量时才会暴露）。给它一点时间。
    let saw_log = wait_for_log(&log, "agent_end:", Duration::from_secs(10));
    let _ = child.kill();
    let _ = child.wait();
    assert!(saw_agent_end, "rpc 应在 stdout 上发出 agent_end");
    assert!(saw_log, "扩展应在限定时间内收到 agent_end");
    let events = read_log(&log);
    assert!(
        events.iter().any(|l| l == "session_start:startup"),
        "扩展应收到 session_start:startup：{events:?}"
    );
    assert!(
        events.iter().any(|l| l.starts_with("agent_end:")),
        "扩展应收到 agent_end：{events:?}"
    );
}

fn have_tmux() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v tmux")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// S2b：TUI 里 `session_start` → `agent_end`，再用 `/resume` 触发 `session_start:resume`。
#[test]
fn tui_dispatches_session_start_and_agent_end() {
    if !have_tmux() {
        eprintln!("跳过：本机没有 tmux");
        return;
    }
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
    let (_mock, base_url) = start_mock("S01");
    write_config(&agent, &base_url);
    let log = tmp.path().join("events.log");
    let session = format!("pi-rs-test-{}", std::process::id());

    let env_prefix = format!(
        "PI_CODING_AGENT_DIR={} PI_CODING_AGENT_SESSION_DIR={} PI_NODE={} PI_EXT_TEST_LOG={} HOME={}",
        agent.display(),
        sess.display(),
        node.display(),
        log.display(),
        tmp.path().join("home").display()
    );
    let cmd = format!(
        "cd {} && {} {}",
        work.display(),
        env_prefix,
        env!("CARGO_BIN_EXE_pi-rs")
    );
    let status = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-x",
            "100",
            "-y",
            "30",
            &cmd,
        ])
        .status()
        .expect("启动 tmux");
    assert!(status.success(), "tmux 会话应能建立");
    let guard = TmuxGuard(session.clone());

    let mut saw_start = wait_for_log(&log, "session_start:startup", Duration::from_secs(60));
    if !saw_start {
        // 首帧尚未就绪时补一次刷新（扩展在宿主启动时就该记录，通常无需输入）。
        std::thread::sleep(Duration::from_secs(2));
        saw_start = wait_for_log(&log, "session_start:startup", Duration::from_secs(10));
    }
    assert!(
        saw_start,
        "TUI 启动应派发 session_start:startup：{:?}",
        read_log(&log)
    );

    // TUI 是默认模式（有 pty 时）；不需要额外开关。
    // TUI 首帧就绪前粘贴会丢输入（实测：直接粘贴时扩展只收到 session_start）。
    std::thread::sleep(Duration::from_secs(3));
    let buffer = format!("{session}-buf");
    let mut agent_end = false;
    for _attempt in 0..3 {
        send_line(&session, &buffer, b"hi\n");
        if wait_for_log(&log, "agent_end:", Duration::from_secs(20)) {
            agent_end = true;
            break;
        }
    }
    assert!(
        agent_end,
        "TUI 一轮结束应派发 agent_end：{:?}",
        read_log(&log)
    );
    // 顺序：session_start 必须在 agent_end 之前。
    let events = read_log(&log);
    let start_at = events.iter().position(|l| l == "session_start:startup");
    let end_at = events.iter().position(|l| l.starts_with("agent_end:"));
    assert!(
        matches!((start_at, end_at), (Some(a), Some(b)) if a < b),
        "session_start 应在 agent_end 之前：{events:?}"
    );

    // `/resume` → `session_start:resume`。
    let mut resumed = false;
    for _attempt in 0..3 {
        send_line(&session, &buffer, b"/resume\n");
        // 选择器：等它画出来再回车选中第一项。
        std::thread::sleep(Duration::from_secs(2));
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", &session, "Enter"])
            .status();
        if wait_for_log(&log, "session_start:resume", Duration::from_secs(10)) {
            resumed = true;
            break;
        }
    }
    assert!(
        resumed,
        "/resume 应派发 session_start:resume：{:?}",
        read_log(&log)
    );
    drop(guard);
}

/// 用 `load-buffer` + `paste-buffer -p`（括号粘贴）往 tmux 会话送一行。
fn send_line(session: &str, buffer: &str, text: &[u8]) {
    let mut paste = Command::new("tmux")
        .args(["load-buffer", "-b", buffer, "-"])
        .stdin(Stdio::piped())
        .spawn()
        .expect("load-buffer");
    paste
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(text)
        .expect("写缓冲");
    let _ = paste.wait();
    let _ = Command::new("tmux")
        .args(["paste-buffer", "-p", "-b", buffer, "-t", session])
        .status();
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", session, "Enter"])
        .status();
}

struct TmuxGuard(String);

impl Drop for TmuxGuard {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &self.0])
            .status();
    }
}
