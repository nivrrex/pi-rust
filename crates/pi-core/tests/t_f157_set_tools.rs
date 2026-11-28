//! `pi.setActiveTools` 的运行期工具集替换（ref `agent-session.ts:1279-1291`）。
//!
//! ref 的 `setActiveToolsByName` 从注册表筛出有效工具、替换 `agent.state.tools`，
//! 下一次请求体与 `toolsAdded` 即反映新集合。
//!
//! 此前 pi-rs 的工具集在 `AgentLoop::new` 之后**不可变**，扩展调用
//! `pi.setActiveTools` 没有任何效果。

use std::sync::Arc;
use std::sync::Mutex;

use futures::future::BoxFuture;
use pi_ai::{
    AssistantEvent, AssistantMessage, Provider, ProviderError, ProviderRequest, StopReason, Stream,
};
use pi_core::agent::{AgentLoop, CollectSink, LoopConfig};
use pi_core::tools::tool::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

struct StubTool {
    name: String,
}

impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.name
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

struct NoopProvider;

impl Provider for NoopProvider {
    fn stream<'a>(
        &'a self,
        _request: ProviderRequest,
    ) -> BoxFuture<'a, Result<Stream, ProviderError>> {
        Box::pin(async move {
            let message = AssistantMessage {
                role: "assistant".to_string(),
                content: Vec::new(),
                api: "stub".to_string(),
                provider: "stub".to_string(),
                model: "mock-model".to_string(),
                usage: Default::default(),
                response_model: None,
                stop_reason: StopReason::Stop,
                error_message: None,
                raw_stop_reason: None,
                end_turn: None,
                response_id: None,
                timestamp: 0,
            };
            let slot = Arc::new(Mutex::new(Some(message)));
            Ok(Stream::with_final_message(
                Box::pin(futures::stream::iter(vec![AssistantEvent::Start])),
                slot,
            ))
        })
    }
}

fn stub(name: &str) -> Arc<dyn Tool> {
    Arc::new(StubTool {
        name: name.to_string(),
    })
}

fn make_loop(tools: Vec<Arc<dyn Tool>>) -> AgentLoop {
    AgentLoop::new(
        Arc::new(NoopProvider),
        tools,
        Arc::new(CollectSink::default()),
        LoopConfig {
            model: "mock-model".to_string(),
            max_completion_tokens: 4096,
            ..LoopConfig::default()
        },
    )
}

#[test]
fn set_tools_replaces_the_tool_set() {
    let mut agent_loop = make_loop(vec![stub("bash"), stub("read")]);
    assert_eq!(agent_loop.tool_names(), vec!["bash", "read"]);

    agent_loop.set_tools(vec![stub("read")]);
    assert_eq!(
        agent_loop.tool_names(),
        vec!["read"],
        "set_tools 之后工具集应只剩 read"
    );

    // 顺序按调用方给的顺序（ref 从注册表按入参顺序筛）。
    agent_loop.set_tools(vec![stub("edit"), stub("bash")]);
    assert_eq!(agent_loop.tool_names(), vec!["edit", "bash"]);
}

/// 空列表 ⇒ 清空（ref `setActiveTools([])` 得到 `tools = []`）。
#[test]
fn set_tools_can_empty_the_set() {
    let mut agent_loop = make_loop(vec![stub("bash")]);
    agent_loop.set_tools(Vec::new());
    assert!(agent_loop.tool_names().is_empty());
}
