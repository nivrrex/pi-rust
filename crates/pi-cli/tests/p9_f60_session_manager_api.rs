//! F60/17：`ctx.sessionManager` 的只读 API 面必须齐备（ref `session-manager.ts:1140-1160`）。
//!
//! 真实使用方 `pi-ai-summary`（用户已装）的页脚第一行取
//! `ctx.sessionManager?.getCwd?.() ?? ctx.cwd ?? ""`（`src/footer-cny.ts:280`）。
//!
//! 七个只读方法：`getCwd` / `getSessionDir` / `getSessionId` / `getSessionFile` /
//! `getBranch` / `getEntries` / `getLeafId`。此前缺 `getCwd` ⇒ 走 `??` 回退到
//! `ctx.cwd`（进程 cwd）。两者多数情况下相同，但语义不同：会话的 `cwd` 来自
//! **会话头**（`--session <file>` / `-c` 恢复时可能不同于进程 cwd），
//! ref 的页脚显示的是会话的那个。

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

fn install(agent: &Path) {
    let target = agent.join("extensions/sm-probe");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body = std::fs::read_to_string(
        repo_root().join("tools/ext-host/fixtures/session-manager-ext.mjs"),
    )
    .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

fn probe_log() -> Option<String> {
    let _node = node_path()?;
    let agent = tempfile::tempdir().expect("agent");
    install(agent.path());
    let log = agent.path().join("probe.log");
    // **先清掉旧日志**：`PI_EXT_TEST_LOG` 是追加写，同一个临时目录里若已有上一轮记录，
    // "去掉 getCwd" 的改前验证会因为旧行而假通过（实测踩过）。
    let _ = std::fs::remove_file(&log);
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("node"));
    std::env::set_var("PI_EXT_TEST_LOG", &log);
    pi_cli::ext_tools::configure_ctx(agent.path(), "tui", true);
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000d1".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();
    for _ in 0..60 {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.contains("typeof:getCwd=") {
            return Some(text);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    std::fs::read_to_string(&log).ok()
}

/// 七个只读方法都必须是函数，且 `getCwd()` 返回非空路径。
///
/// **两项断言必须在同一个用例里**：`PI_EXT_TEST_LOG` 是**进程级**环境变量，
/// 两个 `#[test]` 并行跑会互相覆盖（实测：一条改红、另一条却绿，交替出现）。
/// 这也是本仓"测试专用开关不进进程级环境变量"那条决策的又一个实例。
#[test]
fn session_manager_read_only_api_is_complete() {
    let Some(log) = probe_log() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    for name in [
        "getCwd",
        "getSessionDir",
        "getSessionId",
        "getSessionFile",
        "getBranch",
        "getEntries",
        "getLeafId",
    ] {
        let needle = format!("typeof:{name}=function");
        assert!(
            log.contains(&needle),
            "ctx.sessionManager.{name} 应是函数（ref session-manager.ts:1140-1160）。日志：\n{log}"
        );
    }
    // 取值：`pi-ai-summary` 的页脚直接用它排版，必须是非空路径。
    let line = log
        .lines()
        .find(|line| line.starts_with("value:getCwd="))
        .unwrap_or_else(|| panic!("日志里没有 getCwd 的取值。日志：\n{log}"));
    let value = line.trim_start_matches("value:getCwd=");
    assert!(
        !value.is_empty() && value != "<none>",
        "getCwd() 应返回路径，实际 {value:?}。日志：\n{log}"
    );
}
