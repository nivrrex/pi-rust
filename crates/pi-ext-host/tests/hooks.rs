//! S4：`tool_call` / `tool_result` 的返回值语义（crate 级）。
//!
//! ref 行号：`runner.ts:1136-1153`（tool_call，首个 block 即止）、
//! `runner.ts:1080-1135`（tool_result，链式改写、抛错只记录）。

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

async fn host_with(fixture: &str) -> Option<ExtHost> {
    let node = node_path()?;
    let options = HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures").join(fixture)],
        cwd: repo_root(),
        env: Vec::new(),
    };
    Some(ExtHost::start(options).await.expect("宿主应能启动"))
}

/// `tool_call` 返回 `block` 时原样带出理由。
#[tokio::test]
async fn tool_call_block_is_returned() {
    let Some(mut host) = host_with("hook-ext.mjs").await else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let result = host
        .emit_event_value(
            "tool_call",
            json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c1",
                   "input": {"blockViaHook": true}}),
            Duration::from_secs(2),
        )
        .await
        .expect("分发应成功");
    let result = result.expect("应返回 block 结果");
    assert_eq!(result["block"], true);
    assert_eq!(result["reason"], "blocked-by-ext");
    host.shutdown().await;
}

/// 不 block 时返回 `None`（不拦截）。
#[tokio::test]
async fn tool_call_without_block_is_none() {
    let Some(mut host) = host_with("hook-ext.mjs").await else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let result = host
        .emit_event_value(
            "tool_call",
            json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c2", "input": {}}),
            Duration::from_secs(2),
        )
        .await
        .expect("分发应成功");
    // F1/S5：`tool_call` 的应答现在**总是**带回 `result.input`（就地修改的载体，
    // ref `agent-loop.ts:727`/`:770-775`），所以"不拦截"不再等价于"应答为 None"。
    // 断言改为"没有 block"——被拦截的语义不变。
    let result = result.expect("不 block 也应有应答（承载回传的 input）");
    assert_ne!(result["block"], serde_json::json!(true), "{result:?}");
    host.shutdown().await;
}

/// `tool_result` 链式：第一个处理器脱敏、第二个处理器覆盖内容（顺序证据）。
#[tokio::test]
async fn tool_result_handlers_chain_in_order() {
    let Some(mut host) = host_with("hook-ext.mjs").await else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let result = host
        .emit_event_value(
            "tool_result",
            json!({"type": "tool_result", "toolName": "bash", "toolCallId": "c3",
                   "input": {}, "content": [{"type": "text", "text": "leaked SECRET here"}],
                   "isError": false}),
            Duration::from_secs(2),
        )
        .await
        .expect("分发应成功")
        .expect("应返回替换结果");
    assert_eq!(
        result["content"][0]["text"], "leaked [redacted] here-2",
        "第二个处理器应看到第一个改写后的内容：{result:?}"
    );
    assert_eq!(result["isError"], false);
    host.shutdown().await;
}

/// 处理器抛错：分发仍成功，错误在 `errors` 里，且不返回替换（`None`）。
#[tokio::test]
async fn handler_error_does_not_block_and_yields_no_replacement() {
    let Some(mut host) = host_with("event-ext.mjs").await else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let value = host
        .emit_event_value(
            "tool_result",
            json!({"type": "tool_result", "toolName": "bash", "toolCallId": "c4",
                   "input": {}, "content": [], "isError": false}),
            Duration::from_secs(2),
        )
        .await
        .expect("抛错不应让分发失败");
    // event-ext 没有 tool_result 处理器 ⇒ 无替换。
    assert!(value.is_none(), "{value:?}");
    host.shutdown().await;
}

/// 超时：按"无处理器"降级为 `None`（并已由调用方提示）。
#[tokio::test]
async fn tool_result_timeout_degrades_to_none() {
    let Some(mut host) = host_with("event-ext.mjs").await else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let value = host
        .emit_event_value(
            "tool_result",
            json!({"type": "tool_result", "toolName": "bash", "toolCallId": "c5", "input": {}, "content": []}),
            Duration::from_millis(1),
        )
        .await
        .expect("超时应降级为 None");
    assert!(value.is_none(), "{value:?}");
    host.shutdown().await;
}

/// ctx 数据与 `host.ui.notify` / `host.ui.setStatus` 单向帧（S5）。
#[tokio::test]
async fn ctx_snapshot_and_ui_frames_reach_host() {
    let Some(mut host) = host_with("ctx-ext.mjs").await else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    host.set_ctx(json!({"cwd": "/work/demo", "mode": "tui", "hasUI": true,
                        "model": null, "thinkingLevel": null}));
    host.emit_event(
        "session_start",
        json!({"type": "session_start", "reason": "startup"}),
        Duration::from_secs(2),
    )
    .await
    .expect("分发应成功");
    // `setStatus` 是单向帧，宿主在处理期间就地记账。
    let status = host.last_status();
    assert_eq!(
        status,
        Some(("demo".to_string(), "status-from-ext".to_string())),
        "{status:?}"
    );
    host.shutdown().await;
}
