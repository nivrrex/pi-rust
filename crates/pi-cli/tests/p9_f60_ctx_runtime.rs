//! F60/5：`ctx.abort()` 等会话运行时方法必须存在且可用。
//!
//! 用户已装扩展 `pi-agent-resilience` 在**流卡住**（`stream_stall`）或**检测到文本
//! 循环**时调用 `ctx.abort()`（`index.ts:116/227`），写成
//! `try { ctx.abort() } catch { /* already stopped */ }`。
//!
//! 我们此前**根本没有**这个成员 ⇒ `TypeError: ctx.abort is not a function` 被那个
//! `catch` 吞掉 ⇒ 自动恢复**静默失效**：既不中止回合，也不报错，用户只看到"卡住"。
//! 同一批还缺 `shutdown` / `isIdle` / `isProjectTrusted` / `hasPendingMessages`
//! （ref `extensions/types.ts:336-346`）。
//!
//! 本用例用探针扩展（`tools/ext-host/fixtures/abort-ext.mjs`）把每个成员的
//! `typeof` 与调用结果写进日志，断言：
//! 1. 五个成员都是 `function`（改前全是 `undefined`）；
//! 2. 布尔方法返回布尔值；
//! 3. `ctx.abort()` 调用不抛错且返回 `{ok:true}`（TUI 模式下由主循环消费）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

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

fn install_probe(agent: &Path) {
    let target = agent.join("extensions/abort-probe");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/abort-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 启动宿主并等 `session_start` 的探针日志落盘，返回日志内容。
fn probe_log() -> Option<String> {
    let _node = node_path()?;
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path());
    let log = agent.path().join("probe.log");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("node"));
    std::env::set_var("PI_EXT_TEST_LOG", &log);
    // TUI 模式 + 有 UI：`ctx.abort()` 才有主循环可投递。
    pi_cli::ext_tools::configure_ctx(agent.path(), "tui", true);
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000f6".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();
    // `abort` 走 `host.action.call`（跨进程往返），多给一点余量。
    for _ in 0..60 {
        if log.exists()
            && std::fs::read_to_string(&log)
                .unwrap_or_default()
                .contains("call:abort=")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    std::fs::read_to_string(&log).ok()
}

/// 五个会话运行时方法都必须**存在**（是函数）。
///
/// 改前：`typeof:abort=undefined` 等五行全是 `undefined`，
/// 失败文本即 `ctx.abort 应是函数，实际 undefined`。
#[test]
fn session_runtime_methods_exist() {
    let Some(log) = probe_log() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    for member in [
        "abort",
        "shutdown",
        "isIdle",
        "isProjectTrusted",
        "hasPendingMessages",
    ] {
        let needle = format!("typeof:{member}=function");
        assert!(
            log.contains(&needle),
            "ctx.{member} 应是函数（ref extensions/types.ts:336-346）。日志：\n{log}"
        );
    }
}

/// 布尔方法必须返回布尔值（不是 `undefined`）。
#[test]
fn boolean_methods_return_booleans() {
    let Some(log) = probe_log() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    for prefix in [
        "value:isIdle=",
        "value:isProjectTrusted=",
        "value:hasPendingMessages=",
    ] {
        let line = log
            .lines()
            .find(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("日志里缺 {prefix} 这一行。日志：\n{log}"));
        let value = line.trim_start_matches(prefix);
        assert!(
            value == "true" || value == "false",
            "{prefix} 应返回布尔值，实际 {value:?}。日志：\n{log}"
        );
    }
}

/// `ctx.abort()` 必须能调用并**得到宿主应答**（不是抛错、不是静默丢弃）。
///
/// 改前 `ctx.abort` 是 `undefined` ⇒ 探针记录 `call:abort=throw:…`，用户的
/// `try/catch` 写法下则连记录都没有，自动恢复静默失效。
///
/// 这里不要求 `{ok:true}`：本用例是在**库内**直接启动扩展宿主，没有 TUI 主循环，
/// 因此宿主只能回 `no interactive UI in this mode`——但那仍是**明确应答**，
/// 说明调用真的走到了宿主（`host.action.call` 往返成功）。
/// 「有主循环时中止真的生效」由 `p9_f60_abort_e2e`（真 pty）覆盖。
#[test]
fn abort_call_reaches_the_host() {
    let Some(log) = probe_log() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let line = log
        .lines()
        .find(|line| line.starts_with("call:abort="))
        .unwrap_or_else(|| panic!("日志里没有 abort 调用记录。日志：\n{log}"));
    assert!(
        line.starts_with("call:abort=ok:"),
        "ctx.abort() 不应抛错（应得到宿主应答），实际：{line}。日志：\n{log}"
    );
    // 应答必须是这两种之一，不能是空对象（空对象说明宿主没处理）。
    assert!(
        line.contains("\"ok\":true") || line.contains("no interactive UI in this mode"),
        "宿主应答不合预期（应明确说明成功或无 UI），实际：{line}"
    );
}
