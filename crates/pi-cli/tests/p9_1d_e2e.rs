//! P9-1-D：`tool_call` / `tool_result` 钩子接到 pi-cli 的工具执行上（内置 + 扩展）。
//!
//! 只读临时目录；`PI_NODE` 可覆盖 node 路径。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pi_core::tools::tool::{Tool, ToolContext};

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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn prepare_agent_dir(root: &Path) {
    for (dir, source) in [("hook", "hook-ext.mjs"), ("echo", "echo-ext.mjs")] {
        let target = root.join("extensions").join(dir);
        std::fs::create_dir_all(&target).expect("建扩展目录");
        let body =
            std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures").join(source))
                .expect("读 fixture");
        std::fs::write(target.join("index.mjs"), body).expect("写扩展");
    }
}

fn tools_for(agent_dir: &Path) -> Vec<Arc<dyn Tool>> {
    std::env::set_var("PI_CODING_AGENT_DIR", agent_dir);
    std::env::set_var("PI_NODE", node_path().expect("需要 node"));
    pi_cli::ext_tools::configure_ctx(agent_dir, "print", false);
    pi_cli::print_mode::default_tools(&pi_cli::args::Args::default())
}

fn find(tools: &[Arc<dyn Tool>], name: &str) -> Arc<dyn Tool> {
    tools
        .iter()
        .find(|tool| tool.name() == name)
        .cloned()
        .unwrap_or_else(|| panic!("缺少工具 {name}"))
}

#[tokio::test]
async fn hooks_apply_to_builtin_and_extension_tools() {
    if node_path().is_none() {
        eprintln!("跳过：本机没有 node");
        return;
    }
    let agent = tempfile::tempdir().expect("agent dir");
    prepare_agent_dir(agent.path());
    let work = tempfile::tempdir().expect("work dir");
    let secret_file = work.path().join("note.txt");
    std::fs::write(&secret_file, "line with SECRET inside\n").expect("写文件");

    let tools = tools_for(agent.path());
    let context = ToolContext {
        cwd: work.path().to_path_buf(),
    };
    let cancel = tokio_util::sync::CancellationToken::new();

    // 1) 内置 read：输出经 `tool_result` 钩子脱敏（hook-ext 的两个处理器链式：脱敏 + 追加 -2）。
    let read = find(&tools, "read");
    let result = read
        .execute(
            serde_json::json!({"path": secret_file.display().to_string()}),
            &context,
            &cancel,
        )
        .await;
    let text = result.text_content();
    assert!(text.contains("[redacted]"), "内置工具输出应被脱敏：{text}");
    assert!(!text.contains("SECRET"), "原文不应残留：{text}");
    assert!(text.ends_with("-2"), "第二个处理器也应生效：{text}");

    // 2) 扩展工具 ext_echo：同样经钩子。
    let echo = find(&tools, "ext_echo");
    let result = echo
        .execute(
            serde_json::json!({"text": "SECRET-data"}),
            &context,
            &cancel,
        )
        .await;
    let echo_debug = format!("{result:?}");
    let text = result.text_content();
    assert!(text.contains("[redacted]"), "扩展工具输出应被脱敏：{text}");
    assert!(!text.contains("SECRET"), "原文不应残留：{text}");
    // 落盘形态 = 序列化后的 ToolResult（会话里的 toolResult 由 loop 按它写出）。
    assert!(
        !echo_debug.contains("SECRET"),
        "落盘形态不应含原文：{echo_debug}"
    );

    // 3) tool_call block：拦截后不执行，返回扩展给的理由。
    let result = echo
        .execute(
            serde_json::json!({"text": "x", "blockViaHook": true}),
            &context,
            &cancel,
        )
        .await;
    assert!(result.is_error, "被 block 应标记错误");
    assert_eq!(result.text_content(), "blocked-by-ext");
}
