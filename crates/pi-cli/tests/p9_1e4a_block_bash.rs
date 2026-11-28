//! S2：宿主动作帧的 `bash` 与内置工具走同一条 `HookedTool` 路径 ⇒ `tool_call` 的 `block` 同样生效。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pi_core::tools::tool::{Tool, ToolContext};

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn prepare(agent: &Path) {
    for (dir, source) in [("action", "action-bash-ext.mjs"), ("hook", "hook-ext.mjs")] {
        let target = agent.join("extensions").join(dir);
        std::fs::create_dir_all(&target).expect("建目录");
        let body =
            std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures").join(source))
                .expect("读 fixture");
        std::fs::write(target.join("index.mjs"), body).expect("写扩展");
    }
}

#[tokio::test]
async fn action_frame_bash_is_blocked_by_tool_call_hook() {
    if node_path().is_none() {
        eprintln!("跳过：本机没有 node");
        return;
    }
    let agent = tempfile::tempdir().expect("agent dir");
    prepare(agent.path());
    let work = tempfile::tempdir().expect("work dir");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("需要 node"));
    pi_cli::ext_tools::configure_ctx(agent.path(), "print", false);
    let tools: Vec<Arc<dyn Tool>> =
        pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "ext_action_bash")
        .cloned()
        .expect("应有扩展工具");
    let context = ToolContext {
        cwd: work.path().to_path_buf(),
    };
    let cancel = tokio_util::sync::CancellationToken::new();

    // `hook-ext` 的 tool_call 在参数里带 `blockViaHook` 时 block（理由 `blocked-by-ext`）。
    let result = tool
        .execute(
            serde_json::json!({"command": "echo SHOULD-NOT-RUN", "blockViaHook": true}),
            &context,
            &cancel,
        )
        .await;
    assert!(result.is_error, "被 block 应标记错误");
    assert_eq!(result.text_content(), "blocked-by-ext");
    assert!(
        !result.text_content().contains("SHOULD-NOT-RUN"),
        "block 后不应真的执行命令"
    );
}
