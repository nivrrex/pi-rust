//! `before_agent_start` 的 `systemPromptOptions`（ref `types.ts:738-746`）。
//!
//! 契约：事件里带 `systemPromptOptions`（含 `sections`），处理器的修改**回流**到
//! 系统提示。真机上 `pi-handoff-gate` 依赖它（`index.ts:557`），字段缺失时它抛
//! `Cannot read properties of undefined (reading 'sections')` —— 表现为
//! "扩展事件 before_agent_start 处理器失败"，续跑/提示注入随之失效。
//!
//! 此前 pi-rs 的 payload 只有 `{prompt, systemPrompt}`，没有 `systemPromptOptions`。

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
    let dir = agent.join("extensions/ba-sections");
    std::fs::create_dir_all(&dir).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures").join(fixture))
        .expect("读 fixture");
    std::fs::write(dir.join("index.mjs"), body).expect("写扩展");
}

#[test]
fn before_agent_start_carries_mutable_sections_and_returns_them() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install(agent.path(), "before-agent-start-sections-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "print", false);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    let _ = pi_cli::ext_tools::reload_host(agent.path(), &cwd);

    let result = pi_cli::ext_tools::emit_before_agent_start("hello", "SYSTEM");
    // 扩展只有在拿到 `systemPromptOptions.sections` 时才会写这一节；
    // 字段缺失时它走 `missing-*` 分支、什么都不写。
    let sections = result.sections.expect("before_agent_start 应带回 sections");
    assert_eq!(
        sections.get("handoff-gate-test").and_then(|v| v.as_str()),
        Some("SENTINEL-SECTION-TEXT"),
        "处理器对 sections 的修改应回流（缺失 systemPromptOptions 时这里会是空）：{sections:?}"
    );
}
