//! S5：事件分发起步（`session_start` / `agent_end`）的语义验证。
//!
//! 覆盖：订阅收到、`unsubscribe` 生效、处理器抛错不影响主流程、超时不影响宿主存活。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{ExtHost, HostOptions};
use serde_json::json;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
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

async fn start(node: &std::path::Path, log: &std::path::Path) -> ExtHost {
    let options = HostOptions {
        node: node.to_path_buf(),
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/event-ext.mjs")],
        cwd: repo_root(),
        env: vec![("PI_EXT_TEST_LOG".to_string(), log.display().to_string())],
    };
    ExtHost::start(options).await.expect("宿主应能启动")
}

fn read_log(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// `session_start` 收到一次；处理器里 `unsubscribe()` 之后第二次不再收到。
#[tokio::test]
async fn session_start_is_dispatched_and_unsubscribe_works() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("events.log");
    let mut host = start(&node, &log).await;

    let first = host
        .emit_event(
            "session_start",
            json!({"type": "session_start", "reason": "startup"}),
            Duration::from_secs(2),
        )
        .await
        .expect("分发应成功");
    assert_eq!(first["handled"], 1, "{first:?}");

    let second = host
        .emit_event(
            "session_start",
            json!({"type": "session_start", "reason": "reload"}),
            Duration::from_secs(2),
        )
        .await
        .expect("分发应成功");
    assert_eq!(second["handled"], 0, "退订后不应再收到：{second:?}");

    let lines = read_log(&log);
    assert_eq!(
        lines,
        vec!["session_start:startup".to_string()],
        "{lines:?}"
    );
    host.shutdown().await;
}

/// 处理器抛错：分发本身仍 `Ok`，错误以 `errors[]` 回传，宿主随后仍可正常调用工具。
#[tokio::test]
async fn handler_error_is_reported_but_does_not_kill_host() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("events.log");
    let mut host = start(&node, &log).await;

    let result = host
        .emit_event(
            "agent_end",
            json!({"type": "agent_end", "messages": [{"role": "user"}]}),
            Duration::from_secs(2),
        )
        .await
        .expect("处理器抛错不应让分发失败");
    assert_eq!(result["handled"], 1, "{result:?}");
    let errors = result["errors"].as_array().cloned().unwrap_or_default();
    assert_eq!(errors.len(), 1, "{result:?}");
    assert!(
        errors[0].as_str().unwrap_or_default().contains("boom"),
        "{result:?}"
    );

    // 宿主仍活着：工具可用。
    let pong = host
        .invoke_tool("ext_ping", json!({}), Duration::from_secs(5))
        .await
        .expect("抛错后仍应可调用工具");
    assert_eq!(pong["content"][0]["text"], "pong");
    assert!(read_log(&log).iter().any(|line| line == "agent_end:1"));
    host.shutdown().await;
}

/// 超时：极短超时下分发报 timeout，但宿主不被杀死，后续调用正常。
#[tokio::test]
async fn event_timeout_does_not_kill_host() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("events.log");
    let mut host = start(&node, &log).await;

    let error = host
        .emit_event("slow_event", json!({}), Duration::from_millis(20))
        .await
        .expect_err("20 ms 超时应报错（处理器睡 500 ms）");
    assert!(
        matches!(&error, pi_ext_host::HostError::Protocol(message) if message.contains("timeout")),
        "{error:?}"
    );
    let pong = host
        .invoke_tool("ext_ping", json!({}), Duration::from_secs(5))
        .await;
    assert!(pong.is_ok(), "超时后宿主应仍可用：{pong:?}");
    host.shutdown().await;
}
