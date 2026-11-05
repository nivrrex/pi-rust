//! 集成测试：真实 spawn Node 子进程跑 `tools/ext-host/runner.mjs` + 测试扩展。
//!
//! 覆盖 P9-1-B 的三条硬要求：无 node、子进程崩溃、握手版本不符；
//! 另外验证 `registerTool` 往返与"扩展 stdout 污染不影响控制通道"。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{ExtHost, HostError, HostOptions};
use serde_json::json;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runner() -> PathBuf {
    repo_root().join("tools/ext-host/runner.mjs")
}

fn fixture() -> PathBuf {
    repo_root().join("tools/ext-host/fixtures/echo-ext.mjs")
}

/// 找 node：`command -v node`（测试机器必须有；没有就跳过并打印原因）。
fn node_path() -> Option<String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

fn options(node: &str) -> HostOptions {
    HostOptions {
        node: PathBuf::from(node),
        runner: runner(),
        extensions: vec![fixture()],
        cwd: repo_root(),
        env: Vec::new(),
    }
}

/// 正常路径：注册两个工具、调用回显、调用抛错工具。
#[tokio::test]
async fn registers_tools_and_roundtrips_invocation() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let mut host = ExtHost::start(options(&node)).await.expect("宿主应能启动");
    let names: Vec<String> = host.tools().iter().map(|tool| tool.name.clone()).collect();
    assert!(
        names.contains(&"ext_echo".to_string()),
        "工具列表：{names:?}"
    );
    assert!(
        names.contains(&"ext_sleep".to_string()),
        "工具列表：{names:?}"
    );

    let result = host
        .invoke_tool("ext_echo", json!({"text": "你好"}), Duration::from_secs(10))
        .await
        .expect("调用应成功");
    assert_eq!(result["content"][0]["text"], "echo:你好");

    let failed = host
        .invoke_tool("ext_boom", json!({}), Duration::from_secs(10))
        .await
        .expect("抛错也应回包（isError）");
    assert_eq!(failed["isError"], true);
    assert!(
        failed["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("boom"),
        "{failed:?}"
    );
    host.shutdown().await;
}

/// 扩展 stdout 有噪音（fixture 顶部 console.log）不影响协议：控制通道是 loopback TCP。
#[tokio::test]
async fn extension_stdout_noise_does_not_break_protocol() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let mut host = ExtHost::start(options(&node))
        .await
        .expect("有 stdout 噪音也应成功");
    assert!(host.tools().iter().any(|tool| tool.name == "ext_echo"));
    host.shutdown().await;
}

/// 工具超时：宿主给 50 ms，扩展睡 400 ms ⇒ 超时错误，子进程仍可用。
#[tokio::test]
async fn tool_timeout_is_reported_and_host_survives() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let mut host = ExtHost::start(options(&node)).await.expect("宿主应能启动");
    let error = host
        .invoke_tool("ext_sleep", json!({"ms": 400}), Duration::from_millis(50))
        .await
        .expect_err("应当超时");
    assert!(
        matches!(&error, HostError::Protocol(message) if message.contains("timeout")),
        "{error:?}"
    );
    // 超时后仍能继续调用（迟到的响应按 id 丢弃）。
    let result = host
        .invoke_tool(
            "ext_echo",
            json!({"text": "after"}),
            Duration::from_secs(10),
        )
        .await;
    assert!(result.is_ok(), "超时后宿主应仍可用：{result:?}");
    host.shutdown().await;
}

/// 无 node：`NodeMissing`，不 panic、不挂起。
#[tokio::test]
async fn missing_node_reports_clear_error() {
    let error = ExtHost::start(options("/nonexistent/node-for-pi-ext-host"))
        .await
        .expect_err("没有 node 必须报错");
    assert!(
        matches!(&error, HostError::NodeMissing(path) if path.contains("nonexistent")),
        "{error:?}"
    );
}

/// 子进程被 kill -9：调用返回 `Exited`，不会永久挂起。
#[tokio::test]
async fn killed_child_surfaces_exit() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let mut host = ExtHost::start(options(&node)).await.expect("宿主应能启动");
    let pid = host.pid().expect("应有 pid");
    // 只杀本测试自己启动的子进程（pid 来自 child.id()）。
    let status = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status()
        .expect("kill 命令应可用");
    assert!(status.success(), "kill 失败");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let error = host
        .invoke_tool("ext_echo", json!({"text": "x"}), Duration::from_secs(5))
        .await
        .expect_err("子进程死后调用必须失败");
    assert!(
        matches!(&error, HostError::Exited(_) | HostError::Io(_)),
        "{error:?}"
    );
}

/// 协议版本不符：子进程声明 999 ⇒ 握手拒绝，错误里带对方版本。
#[tokio::test]
async fn version_mismatch_is_rejected() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let mut opts = options(&node);
    opts.env
        .push(("PI_EXT_PROTOCOL".to_string(), "999".to_string()));
    let error = ExtHost::start(opts).await.expect_err("版本不符必须拒绝");
    assert!(
        matches!(
            &error,
            HostError::Handshake(pi_ext_host::HandshakeError::VersionMismatch { theirs: 999 })
        ),
        "{error:?}"
    );
}

/// `ExtTool` 包装：schema/描述来自注册信息，执行走宿主。
#[tokio::test]
async fn ext_tool_adapter_executes_through_host() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = pi_ext_host::HostHandle::start(options(&node)).expect("actor 应能启动");
    let spec = host
        .tools()
        .iter()
        .find(|tool| tool.name == "ext_echo")
        .cloned()
        .expect("应有 ext_echo");
    let tool = pi_ext_host::ExtTool::new(spec, host.clone());
    use pi_core::tools::tool::{Tool, ToolContext};
    assert_eq!(tool.name(), "ext_echo");
    assert_eq!(tool.schema()["type"], "object");
    let context = ToolContext { cwd: repo_root() };
    let cancel = tokio_util::sync::CancellationToken::new();
    let result = tool
        .execute(json!({"text": "via-tool"}), &context, &cancel)
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(result.text_content(), "echo:via-tool");
    host.shutdown();
}
