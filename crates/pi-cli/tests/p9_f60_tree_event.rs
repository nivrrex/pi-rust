//! F60/7：树导航后必须派发 `session_tree`（ref `agent-session.ts:3762-3768`）。
//!
//! `pi-handoff-gate`（用户已装）在 `session_tree` 里从 `getBranch()` **重建 todo 清单**
//! （`index.ts:305-308`，注释写明「随会话分支；压缩不影响」）。此前 pi-rs 的
//! `/tree` 只切叶子、**从不派发**该事件 ⇒ 换分支后清单停在旧分支，
//! 用户看到"分支换了但清单没跟着变"。
//!
//! 本用例验证**派发链路**本身可用：宿主能收到 `session_tree` 并把载荷原样交给扩展
//! （`newLeafId` / `oldLeafId` 都到位）。树导航的 UI 操作序列需要构造带分支的会话
//! 并逐键驱动 `/tree` 面板，属另一批任务（已登记在 `notes/p9-f60/plan.md`）。

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
    let target = agent.join("extensions/tree-probe");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body =
        std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/tree-event-ext.mjs"))
            .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 宿主把 `session_tree` 交给扩展，载荷里的 `newLeafId` / `oldLeafId` 原样到达。
#[test]
fn session_tree_reaches_the_extension() {
    let Some(_node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install(agent.path());
    let log = agent.path().join("probe.log");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("node"));
    std::env::set_var("PI_EXT_TEST_LOG", &log);
    pi_cli::ext_tools::configure_ctx(agent.path(), "tui", true);
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000f7".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();

    pi_cli::ext_tools::emit_event(
        "session_tree",
        serde_json::json!({
            "type": "session_tree",
            "newLeafId": "leaf-new",
            "oldLeafId": "leaf-old",
            "summaryEntry": serde_json::Value::Null,
            "fromExtension": false,
        }),
    );

    let body = {
        let mut text = String::new();
        for _ in 0..50 {
            text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.contains("session_tree:") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        text
    };
    assert!(
        body.contains("session_tree:new=leaf-new:old=leaf-old"),
        "扩展应收到带 newLeafId/oldLeafId 的 session_tree。日志：\n{body}"
    );
}
