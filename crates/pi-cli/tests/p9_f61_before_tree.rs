//! F61/38：`session_before_tree`（ref `agent-session.ts:3641-3656`）。
//!
//! 此前 pi-rs 完全不派发该事件（LIMITATIONS 27a）。扩展据此可以：
//! - `{cancel:true}` ⇒ 放弃本次 `/tree` 导航；
//! - `{summary}` ⇒ 直接给出分支摘要，**省掉一次模型调用**（ref 的 `fromExtension`）。
//!
//! 本用例验证 `ext_tools::emit_session_before_tree` 的三个分支（取消 / 给摘要 /
//! 不介入），那正是 TUI 分派处依赖的契约；`/tree` 交互路径由既有 T52 场景覆盖。

use std::path::Path;

fn node_path() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| std::path::PathBuf::from(text))
}

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 安装指定 fixture 作为扩展。
///
/// 用**不同文件**而不是同一夹具 + 环境变量分流：扩展宿主进程只继承白名单环境
/// （`ext_tools.rs` 显式列出的那几项），自定义变量传不进去，夹具读到的永远是
/// `undefined`（实测三路都成了"不介入"）。
fn install_probe(agent: &Path, fixture: &str) {
    let target = agent.join("extensions/before-tree");
    std::fs::create_dir_all(&target).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures").join(fixture))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 装 fixture **并重启宿主**，然后派发一次。
///
/// 必须重启：扩展宿主是进程级单例（`HOST`），同一进程里多次调用会复用第一个
/// 宿主（连带它的 fixture）⇒ 后面几路测的根本不是自己的扩展。
/// 最初没重启，三路结果完全一致（全都不介入），正是这个原因。
fn probe(fixture: &str) -> Option<pi_cli::ext_tools::BeforeTree> {
    let node = node_path()?;
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), fixture);
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    // 先让宿主启动成功（`extension_tools` 首次调用会拉起 node），再重启到本路的
    // agentDir——直接 `reload_host` 时若还没有宿主，`start_host` 拿到的上下文不完整。
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");
    Some(pi_cli::ext_tools::emit_session_before_tree(
        Some("target-1"),
        Some("from-1"),
    ))
}

/// 三个分支必须**串行**（一个 `#[test]` 内依次跑）。
///
/// 扩展宿主是进程级单例，三个 `#[test]` 并行时会互相抢句柄（实测并行下
/// "不介入"那条失败）。这与 `p9_f1_tool_call_input` 的教训完全相同。
#[test]
fn session_before_tree_semantics() {
    // 取消。
    let Some(cancel) = probe("before-tree-ext.mjs") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        cancel.cancel,
        "扩展返回 {{cancel:true}} 时导航应被取消：{cancel:?}"
    );
    assert_eq!(cancel.summary, None, "取消时不应同时给摘要：{cancel:?}");

    // 给摘要：省掉模型调用，且不取消。
    let summary = probe("before-tree-summary-ext.mjs").expect("node 在");
    assert!(!summary.cancel, "给了摘要不应同时取消：{summary:?}");
    assert_eq!(summary.summary.as_deref(), Some("EXTENSION-SUMMARY"));

    // 不介入：两者都为空（等同 ref 无 handler 的行为）。
    let noop = probe("before-tree-noop-ext.mjs").expect("node 在");
    assert!(!noop.cancel, "默认不应取消：{noop:?}");
    assert_eq!(noop.summary, None, "默认不应给摘要：{noop:?}");

    // `session_before_switch` 的 `{cancelled:true}` 也必须生效。
    //
    // 与 `session_before_tree` 是**同一个缺陷**：runner 的通用分支此前把返回值丢掉，
    // 于是这个取消信号从未生效过（`emit_session_before_switch` 永远 `false`）。
    // 三个场景都必须在**同一个 `#[test]` 内串行**——扩展宿主是进程级单例，
    // 多个测试并行会互相抢句柄（实测全量并行时第一条就失败）。
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "before-switch-cancel-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");
    assert!(
        pi_cli::ext_tools::emit_session_before_switch("new", None),
        "扩展返回 {{cancelled:true}} 时应取消切换"
    );
}
