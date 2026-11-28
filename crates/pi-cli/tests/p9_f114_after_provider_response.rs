//! F114：`after_provider_response` 的载荷必须是**真实 HTTP 响应**的 status 与 headers
//! （ref `sdk.ts:357-362` → `openai-completions.ts:378` 的 `onResponse`）。
//!
//! F111 当时是手工跑一次性探针确认的，没有留下自动验证。这条用例把它固化：
//! 用 mock 跑一轮 `--print`，断言探针收到的 `status` 是 200、`headers` 里带
//! `content-type: text/event-stream`。
//!
//! **不覆盖错误响应**：429 之类不会产生该事件——ref 的 `onResponse` 在
//! `retryProviderRequest` **之后**调用（`openai-completions.ts:369-378`），
//! 只有拿到响应才触发；pi-rs 的 `stream()` 同样在拿到响应前就返回 `Err`。

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
    let dir = agent.join("extensions/after-response");
    std::fs::create_dir_all(&dir).expect("建扩展目录");
    let body = std::fs::read_to_string(
        repo_root().join("tools/ext-host/fixtures/after-response-probe-ext.mjs"),
    )
    .expect("读夹具");
    std::fs::write(dir.join("index.mjs"), body).expect("写扩展");
    std::fs::write(
        dir.join("package.json"),
        r#"{"type":"module","name":"after-response"}"#,
    )
    .expect("写 package.json");
}

#[test]
fn payload_carries_the_real_http_status_and_headers() {
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
    let mut mock = Command::new(repo_root().join("target/debug/mock-provider"))
        .current_dir(repo_root())
        .arg("scenarios/S01.json")
        .arg("--port")
        .arg("0")
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
    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("hi")
        .env("HOME", &home)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .output()
        .expect("跑 pi-rs");
    let _ = mock.kill();
    let _ = mock.wait();
    assert!(
        output.status.success(),
        "pi-rs 应成功：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = std::fs::read_to_string(agent.join("after-response.log")).unwrap_or_default();
    assert!(
        !log.is_empty(),
        "扩展应收到 `after_provider_response`（探针日志为空）"
    );
    let first: serde_json::Value =
        serde_json::from_str(log.lines().next().expect("至少一行")).expect("解析探针日志");
    assert_eq!(
        first.get("status").and_then(serde_json::Value::as_u64),
        Some(200),
        "status 应是真实 HTTP 状态码：{first}"
    );
    let headers = first.get("headers").expect("应有 headers");
    assert_eq!(
        headers
            .get("content-type")
            .and_then(serde_json::Value::as_str),
        Some("text/event-stream"),
        "应带上真实响应头：{headers}"
    );
}

/// F115：扩展加载失败在 TUI 模式下**不走 stderr**——那里已由 `[Extension issues]`
/// 段呈现，而 stderr 的每一行都会把 inline 首屏挤下去（与 F63/K、F75 同类问题）。
///
/// `--print` 模式仍走 stderr（没有 TUI 通道，不写就等于丢掉信息）。
#[test]
fn load_errors_do_not_leak_to_stderr_in_tui_mode() {
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
    // 一个加载就抛错的扩展。
    let bad = agent.join("extensions/bad");
    std::fs::create_dir_all(&bad).expect("建扩展目录");
    std::fs::write(
        bad.join("index.mjs"),
        "export default function () { throw new Error(\"boom-on-load\"); }\n",
    )
    .expect("写扩展");
    std::fs::write(
        bad.join("package.json"),
        r#"{"type":"module","name":"bad"}"#,
    )
    .expect("写 package.json");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}"#,
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
        r#"[{"waitStable":{"quietMs":2500,"timeoutMs":30000}},{"text":"\u0004"},{"delayMs":800}]"#,
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
    assert!(output.status.success(), "pi-rs 应正常退出");
    let raw = std::fs::read_to_string(tmp.path().join("raw.bin")).unwrap_or_default();
    let stripped = {
        // 粗剥 CSI/OSC，只留可见文本。
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\u{1b}' {
                match chars.peek() {
                    Some('[') => {
                        chars.next();
                        for next in chars.by_ref() {
                            if next.is_ascii_alphabetic() {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        chars.next();
                        for next in chars.by_ref() {
                            if next == '\u{7}' {
                                break;
                            }
                        }
                    }
                    _other => {}
                }
                continue;
            }
            out.push(ch);
        }
        out
    };
    assert!(
        stripped.contains("[Extension issues]"),
        "TUI 应把扩展问题汇总成 `[Extension issues]` 段"
    );
    assert!(
        !stripped.contains("扩展加载失败"),
        "TUI 模式下不该把加载失败打到 stderr（会挤掉首屏）"
    );
}
