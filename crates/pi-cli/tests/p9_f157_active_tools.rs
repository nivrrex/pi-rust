//! `pi.setActiveTools`（ref `agent-session.ts:1279-1291` 的 `setActiveToolsByName`）。
//!
//! 核心规则抽在 `select_active_tools` 里：按入参顺序、只在已知集合里取、未知名字忽略。
//! 此前工具集在构造期固定（`Vec` 不可变），扩展调用后没有任何效果。

use std::sync::Arc;

use futures::future::BoxFuture;
use pi_cli::rpc_session::select_active_tools;
use pi_core::tools::tool::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

struct NamedTool(&'static str);

impl Tool for NamedTool {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "stub"
    }
    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| serde_json::json!({"type": "object"}))
    }
    fn execute<'a>(
        &'a self,
        _arguments: Value,
        _context: &'a ToolContext,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move { ToolResult::text(String::new()) })
    }
}

fn known() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(NamedTool("bash")),
        Arc::new(NamedTool("read")),
        Arc::new(NamedTool("edit")),
    ]
}

fn names(tools: &[Arc<dyn Tool>]) -> Vec<String> {
    tools.iter().map(|tool| tool.name().to_string()).collect()
}

#[test]
fn keeps_only_known_tools_in_input_order() {
    let picked = select_active_tools(
        &known(),
        &[
            "edit".to_string(),
            "not-a-tool".to_string(),
            "bash".to_string(),
        ],
    );
    assert_eq!(
        names(&picked),
        vec!["edit", "bash"],
        "顺序随入参、未知名字忽略"
    );
}

#[test]
fn empty_input_empties_the_set() {
    assert!(select_active_tools(&known(), &[]).is_empty());
}

/// 重复名字会重复出现——ref 也是遍历入参逐个推入（`agent-session.ts:1283-1288`）。
#[test]
fn duplicates_are_preserved() {
    let picked = select_active_tools(&known(), &["read".to_string(), "read".to_string()]);
    assert_eq!(names(&picked), vec!["read", "read"]);
}
