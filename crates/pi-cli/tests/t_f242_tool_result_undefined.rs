//! `tool_result` 处理器只观察、不返回值时必须跳过，不能崩（F242）。
//!
//! ref `extensions/runner.ts:1091` 的语义是 `if (!handlerResult) continue;` ——
//! 处理器返回 `undefined` 表示"不改写"。pi-rs 的 `runner.mjs` 少了这句，
//! 于是任何"只看看"的 `tool_result` 处理器都会让宿主读到
//! `undefined.content`，报
//! `Cannot read properties of undefined (reading 'content')`，
//! 该扩展的 tool_result 钩子整体失效（真机反馈，Windows 版同样）。
//!
//! 这里跑一次真实的 bash 工具调用，装两个扩展：
//! ① `observer`：什么都不返回（复现上述缺陷）；
//! ② `rewriter`：把内容改写成标记串（证明链条没被打断）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct MockGuard(std::process::Child);
impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_mock(log: &std::path::Path) -> (MockGuard, u16) {
    use std::io::Write;
    let binary = repo_root().join("target/debug/mock-provider");
    assert!(binary.exists(), "先构建 mock-provider");
    let mut child = Command::new(binary)
        .current_dir(repo_root())
        .arg("scenarios/S05.json")
        .arg("--port")
        .arg("0")
        .arg("--request-log")
        .arg(log)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mock 可启动");
    let stdout = child.stdout.take().expect("stdout");
    let mut lines = BufReader::new(stdout).lines();
    let mut port = None;
    for line in lines.by_ref() {
        let line = line.expect("读 stdout");
        if let Some(rest) = line.strip_prefix("listening ") {
            port = rest
                .rsplit(':')
                .next()
                .and_then(|text| text.parse::<u16>().ok());
            break;
        }
    }
    let _ = std::io::stdout().flush();
    (MockGuard(child), port.expect("listening"))
}

#[test]
fn observing_tool_result_handler_does_not_break_the_chain() {
    let tmp = tempfile::tempdir().expect("tmp");
    let agent = tmp.path().join("agent");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(agent.join("extensions/observer")).expect("observer 目录");
    std::fs::create_dir_all(agent.join("extensions/rewriter")).expect("rewriter 目录");
    std::fs::create_dir_all(&work).expect("work");

    std::fs::write(
        agent.join("extensions/observer/package.json"),
        r#"{"type":"module","name":"observer"}"#,
    )
    .expect("observer package.json");
    std::fs::write(
        agent.join("extensions/observer/index.js"),
        // 只观察、**不返回任何值** —— 这就是触发缺陷的写法。
        r#"
export default function (pi) {
  pi.on("tool_result", (event) => {
    if (event.toolName === undefined) throw new Error("payload 缺 toolName");
  });
}
"#,
    )
    .expect("observer index.js");

    std::fs::write(
        agent.join("extensions/rewriter/package.json"),
        r#"{"type":"module","name":"rewriter"}"#,
    )
    .expect("rewriter package.json");
    std::fs::write(
        agent.join("extensions/rewriter/index.js"),
        r#"
export default function (pi) {
  pi.on("tool_result", (event) => {
    if (event.toolName !== "bash") return;
    return { content: [{ type: "text", text: "REWRITTEN-BY-EXTENSION" }] };
  });
}
"#,
    )
    .expect("rewriter index.js");

    let request_log = tmp.path().join("requests.jsonl");
    let (mock, port) = start_mock(&request_log);
    std::fs::write(
        agent.join("models.json"),
        format!(
            r#"{{"providers":{{"acme":{{"apiKey":"k","models":[{{"id":"mock-model","provider":"acme","api":"openai-completions","baseUrl":"http://127.0.0.1:{port}/v1"}}]}}}}}}"#
        ),
    )
    .expect("models.json");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"acme","defaultModel":"mock-model"}"#,
    )
    .expect("settings.json");

    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("HOME", tmp.path())
        .args(["--no-session", "-p", "run echo hi"])
        .output()
        .expect("运行 pi-rs");
    drop(mock);

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !text.contains("Cannot read properties of undefined"),
        "只观察的处理器不得让宿主崩：{text:.800}"
    );
    assert!(
        !text.contains("tool_result 处理器失败"),
        "不应报告 tool_result 处理器失败：{text:.800}"
    );
    // 改写结果进的是**下一次请求**，不在打印输出里 ⇒ 用 mock 记录的请求体断言。
    let requests = std::fs::read_to_string(&request_log).unwrap_or_default();
    assert!(
        requests.contains("REWRITTEN-BY-EXTENSION"),
        "后续处理器的改写必须仍然生效（链条不能被打断）：{requests:.800}"
    );
}
