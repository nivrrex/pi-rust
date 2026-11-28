//! F106：`agent_before_settle` 返回 `continue: true` 时 TUI 必须**真的再跑一轮**
//! （ref `agent-session.ts:1546-1549`）。
//!
//! 判据：续跑会把扩展注入的 `entries` 并入上下文再请求 provider，所以 mock 收到的
//! **请求体里会出现**那条催促文本。不能用"日志行数"——mock 的 `--request-log`
//! 每条事件都写一行，行数与请求数无关。

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn install_probe(agent: &Path) {
    let dir = agent.join("extensions/settle-continue");
    std::fs::create_dir_all(&dir).expect("建扩展目录");
    let body = std::fs::read_to_string(
        repo_root().join("tools/ext-host/fixtures/settle-continue-ext.mjs"),
    )
    .expect("读夹具");
    std::fs::write(dir.join("index.mjs"), body).expect("写扩展");
}

#[test]
fn tui_continues_when_before_settle_asks() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    install_probe(&agent);
    let calls_log = agent.join("settle-calls.log");
    let req_log = tmp.path().join("req.ndjson");
    let mut mock = Command::new(repo_root().join("target/debug/mock-provider"))
        .current_dir(repo_root())
        .arg("scenarios/S01.json")
        .arg("--port")
        .arg("0")
        .arg("--request-log")
        .arg(&req_log)
        .env("MOCK_VERBOSE", "1")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("启动 mock-provider");
    let base_url = {
        use std::io::BufRead;
        let stdout = mock.stdout.take().expect("stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
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
        format!("http://127.0.0.1:{}/v1", found.expect("mock 端口"))
    };
    std::fs::write(
        agent.join("models.json"),
        format!(
            r#"{{"providers":{{"p":{{"baseUrl":"{base_url}","api":"openai-completions","apiKey":"k","models":[{{"id":"m1","reasoning":false}}]}}}}}}"#
        ),
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"m1"}"#,
    )
    .expect("写 settings");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        concat!(
            r#"[{"waitStable":{"quietMs":2500,"timeoutMs":60000}}"#,
            r#",{"text":"hi"},{"text":"\r"},{"delayMs":6000}"#,
            r#",{"text":"\u0004"},{"delayMs":1200}]"#,
        ),
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
        .env("HOME", &home)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .output()
        .expect("跑 pty");
    let _ = mock.kill();
    let _ = mock.wait();
    assert!(
        output.status.success(),
        "pi-rs 应正常退出：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = std::fs::read_to_string(&req_log).unwrap_or_default();
    let calls = std::fs::read_to_string(&calls_log).unwrap_or_default();
    assert!(
        requests.contains("CONTINUE-NUDGE-1"),
        "续跑应把扩展注入的 entries 带进下一次请求；settle 调用日志：{calls:?}"
    );
    // 续跑 2 次 ⇒ 共 3 次 settle；若外层循环空转会是十几次（此前踩过）。
    assert!(
        calls.lines().count() == 3,
        "settle 应恰好 3 次（首轮 + 2 次续跑），多于此说明外层循环空转：{calls:?}"
    );
}
