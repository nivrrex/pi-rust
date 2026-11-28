//! F60/20：`pi.events` 必须是**真的**共享事件总线（ref `event-bus.ts:3-31`）。
//!
//! 已装扩展用它做跨扩展协作：
//! - `pi-handoff-gate` 订阅 `RESILIENCE_PRESENT_CHANNEL` / `RESILIENCE_AUTO_RESUME_CHANNEL`
//!   （`index.ts:235-240`），与 `pi-agent-resilience` 约定"谁处理这个空响应边界"；
//! - `pi-agent-resilience` 在 `session_start` 时 `emit` 广播自己的存在；
//! - `pi-interact` ↔ `pi-remote-control` 用 `pi.remote-control.ui.probe.v1` 同步握手。
//!
//! 此前 runner 的 `events` 是 `{ publish(){}, on(){ return () => {} } }`：
//! **方法名都不对**（ref 是 `emit`/`on`）且全是空实现 ⇒ 扩展调 `pi.events.emit(...)`
//! 直接 `TypeError`，上面所有协作静默失效。

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
    let target = agent.join("extensions/events-probe");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body =
        std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/events-bus-ext.mjs"))
            .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 四段断言在**同一个用例**里（`PI_EXT_TEST_LOG` 是进程级环境变量，多用例并行会互相覆盖）。
#[test]
fn extension_event_bus_delivers_messages() {
    let Some(_node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install(agent.path());
    let log = agent.path().join("probe.log");
    // 追加写：先删旧日志，避免"改前应红"假通过。
    let _ = std::fs::remove_file(&log);
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("node"));
    std::env::set_var("PI_EXT_TEST_LOG", &log);
    pi_cli::ext_tools::configure_ctx(agent.path(), "tui", true);
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000d3".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();

    let body = {
        let mut text = String::new();
        for _ in 0..60 {
            text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.contains("seen=") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        text
    };
    // 1. 方法名必须是 ref 的 `emit` / `on`（此前是 `publish`）。
    assert!(
        body.contains("typeof:emit=function"),
        "pi.events.emit 应是函数（ref event-bus.ts:4）。日志：\n{body}"
    );
    assert!(
        body.contains("typeof:on=function"),
        "pi.events.on 应是函数（ref event-bus.ts:5）。日志：\n{body}"
    );
    // 2. emit 的消息必须真的送到 on 注册的处理器。
    assert!(
        body.contains("received:hello"),
        "pi.events.emit 的消息应送达订阅者。日志：\n{body}"
    );
    // 3. 退订之后不应再收到。
    assert!(
        !body.contains("received:after-unsubscribe"),
        "退订后不应再收到消息。日志：\n{body}"
    );
    // 4. 只收到一次（退订生效的计数证据）。
    assert!(
        body.contains("seen=1"),
        "订阅者应只收到 1 条（退订生效）。日志：\n{body}"
    );
}
