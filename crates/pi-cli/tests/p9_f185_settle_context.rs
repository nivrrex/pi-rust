//! `agent_before_settle` 必须带 `context.llmMessages`（ref `types.ts:797-819`）。
//!
//! 两个核心扩展都依赖它：`pi-handoff-gate` 判断"最后一条是否空 assistant"，
//! `pi-agent-resilience` 同理。此前 pi-rs 的载荷只有 `{type, outcome}`，
//! 真机上表现为每轮都刷一行
//! `扩展事件 agent_before_settle 处理器失败：Cannot read properties of undefined (reading 'llmMessages')`，
//! 而这两个扩展的续跑/空回复保护因此完全不工作。
//!
//! 顺带钉住 `entries`/`continue` 也在载荷里（`BoundaryState` 的其余字段）。

use std::path::{Path, PathBuf};

fn node_path() -> Option<PathBuf> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn install(agent: &Path, fixture: &str) {
    let dir = agent.join("extensions/settle-context");
    std::fs::create_dir_all(&dir).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures").join(fixture))
        .expect("读 fixture");
    std::fs::write(dir.join("index.mjs"), body).expect("写扩展");
}

#[test]
fn before_settle_carries_context_llm_messages() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install(agent.path(), "settle-context-ext.mjs");
    let log = agent.path().join("probe.log");
    std::env::set_var("PI_SETTLE_PROBE_LOG", log.display().to_string());
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "print", false);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    let _ = pi_cli::ext_tools::reload_host(agent.path(), &cwd);

    // 给两条消息，验证它们出现在 `context.llmMessages` 里。
    let messages = vec![pi_ai::message::Message::User {
        content: pi_ai::UserContent::Text("你好".to_string()),
        timestamp: 0,
        extra: serde_json::Map::new(),
    }];
    let outcome = pi_cli::ext_tools::emit_agent_before_settle_with_messages("completed", &messages);
    // 扩展侧只要能读到数组就算通过；数量由实现决定，这里断言至少有 1 条。
    let _ = outcome;

    // 宿主不取命令返回值（与既有约定一致），改为读夹具写的日志。
    let _ = pi_cli::ext_tools::invoke_command_text_for_test("settle-context");
    let reported = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .trim()
        .to_string();
    assert!(
        reported.starts_with("ok:"),
        "扩展应看到 context.llmMessages；实际 {reported:?}"
    );
    let count: usize = reported[3..].parse().unwrap_or(0);
    assert!(count >= 1, "llmMessages 不应为空：{reported}");
}
