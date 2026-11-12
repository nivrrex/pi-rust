//! F55/A3：`HookedTool::prepare_arguments` 必须**委托给内层工具**（改前必失败）。
//!
//! ref `harness/tools/tool.ts:41-52`：`invoke` 在**校验之前**调用 `prepareArguments`。
//! 钩子装饰器只加 `tool_call`/`tool_result` 事件，不该改变参数归一。若不委托，
//! `edit` 的 legacy `{path, oldText, newText}` 在**带扩展**时仍 `Validation failed`，
//! 而不带扩展时正常（真实会话踩过）。
//!
//! 本用例起真实扩展宿主（`hook-ext.mjs`）拿 `HostHandle`，再用它包一个会做归一的
//! 假 `edit` 工具——所以测的是**真实 `HookedTool` 实现**，不是复刻逻辑。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pi_core::tools::tool::{Tool, ToolContext};
use serde_json::json;

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
    let target = agent.join("extensions").join("hook");
    std::fs::create_dir_all(&target).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/hook-ext.mjs"))
        .expect("读 hook-ext fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 与 `EditTool` 同形的归一：legacy 顶层 `oldText`/`newText` 折成 `edits` 数组。
struct Normalizing;

impl Tool for Normalizing {
    fn name(&self) -> &'static str {
        "edit"
    }
    fn description(&self) -> &str {
        "test double"
    }
    fn schema(&self) -> &serde_json::Value {
        static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| json!({ "type": "object" }))
    }
    fn prepare_arguments(&self, arguments: serde_json::Value) -> serde_json::Value {
        let mut value = arguments;
        if value.get("edits").is_none() {
            if let (Some(old), Some(new)) =
                (value.get("oldText").cloned(), value.get("newText").cloned())
            {
                value["edits"] = json!([{ "oldText": old, "newText": new }]);
            }
        }
        value
    }
    fn execute<'a>(
        &'a self,
        _arguments: serde_json::Value,
        _context: &'a ToolContext,
        _cancel: &'a tokio_util::sync::CancellationToken,
    ) -> futures::future::BoxFuture<'a, pi_core::tools::tool::ToolResult> {
        Box::pin(async move { pi_core::tools::tool::ToolResult::text("ok") })
    }
}

#[tokio::test]
async fn hooked_tool_delegates_prepare_arguments() {
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
    // `default_tools` 会启动宿主（`HOST.get_or_init`）并包一层 `HookedTool`；
    // 其中任意一个都持有同一个 `HostHandle`，取出来给我们的假工具用。
    let tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    assert!(!tools.is_empty(), "应有内置工具");

    // 通过公开入口拿宿主：`wrap_with_hooks` 用的正是 `HOST` 里那一个。
    let hooked = pi_cli::ext_tools::wrap_with_hooks(vec![Arc::new(Normalizing) as Arc<dyn Tool>]);
    let tool = hooked.first().expect("应有被装饰的工具");
    let prepared = tool.prepare_arguments(json!({
        "path": "alpha.txt",
        "oldText": "A",
        "newText": "B",
    }));
    assert!(
        prepared.get("edits").is_some(),
        "HookedTool 必须委托内层的 prepare_arguments（legacy 参数要折成 edits）：{prepared}"
    );
    assert_eq!(
        prepared["edits"],
        json!([{ "oldText": "A", "newText": "B" }]),
        "折出的 edits 必须与内层一致"
    );
    let _ = work;
}
