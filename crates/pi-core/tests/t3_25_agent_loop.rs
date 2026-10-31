//! P3.4：agent loop 的 I/O 驱动（事件顺序、并行结果顺序、`length` 特判、abort）。
//!
//! 用**脚本化 provider**（预置每轮的最终助手消息）与桩工具驱动，不联网、不起进程。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use pi_ai::message::{Message, ToolResultContent};
use pi_ai::{
    AssistantEvent, AssistantMessage, Content, Provider, ProviderError, ProviderRequest,
    StopReason, Stream, Usage,
};
use pi_core::agent::{AgentEvent, AgentLoop, CollectSink, LoopConfig};
use pi_core::tools::tool::{Tool, ToolContext, ToolResult};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

fn assistant(stop: StopReason, content: Vec<Content>) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".to_string(),
        content,
        api: "openai-completions".to_string(),
        provider: "mock".to_string(),
        model: "mock-model".to_string(),
        response_model: None,
        usage: Usage::default(),
        stop_reason: stop,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 0,
    }
}

fn tool_call(id: &str, name: &str) -> Content {
    Content::ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: serde_json::Map::new(),
        thought_signature: None,
    }
}

/// 按顺序吐出预置的最终助手消息。
struct ScriptedProvider {
    turns: Mutex<VecDeque<AssistantMessage>>,
}

impl Provider for ScriptedProvider {
    fn stream<'a>(
        &'a self,
        _request: ProviderRequest,
    ) -> BoxFuture<'a, Result<Stream, ProviderError>> {
        Box::pin(async move {
            let message = self
                .turns
                .lock()
                .ok()
                .and_then(|mut guard| guard.pop_front())
                .unwrap_or_else(|| assistant(StopReason::Stop, Vec::new()));
            let slot = Arc::new(Mutex::new(Some(message)));
            Ok(Stream::with_final_message(
                Box::pin(futures::stream::iter(vec![AssistantEvent::Start])),
                slot,
            ))
        })
    }
}

/// 桩工具：可设置耗时与返回文本。
struct StubTool {
    name: String,
    delay: Duration,
    text: String,
    terminate: bool,
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
        SCHEMA.get_or_init(|| json!({"type": "object"}))
    }
    fn execute<'a>(
        &'a self,
        _arguments: Value,
        _context: &'a ToolContext,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            tokio::time::sleep(self.delay).await;
            let mut result = ToolResult::text(self.text.clone());
            result.terminate = self.terminate;
            result
        })
    }
}

fn stub(name: &str, delay_ms: u64, text: &str) -> Arc<dyn Tool> {
    Arc::new(StubTool {
        name: name.to_string(),
        delay: Duration::from_millis(delay_ms),
        text: text.to_string(),
        terminate: false,
    })
}

fn kinds(events: &[AgentEvent]) -> Vec<&'static str> {
    events.iter().map(AgentEvent::kind).collect()
}

async fn run(
    turns: Vec<AssistantMessage>,
    tools: Vec<Arc<dyn Tool>>,
    cancel: CancellationToken,
) -> (Vec<AgentEvent>, Vec<Message>) {
    let sink = Arc::new(CollectSink::default());
    let provider = Arc::new(ScriptedProvider {
        turns: Mutex::new(turns.into()),
    });
    let agent_loop = AgentLoop::new(
        provider,
        tools,
        sink.clone(),
        LoopConfig {
            model: "mock-model".to_string(),
            max_completion_tokens: 4096,
            ..LoopConfig::default()
        },
    );
    let transcript = agent_loop
        .run(
            vec![Message::User {
                content: pi_ai::UserContent::Text("hi".to_string()),
                timestamp: 0,
                extra: serde_json::Map::new(),
            }],
            &|_| Vec::new(),
            &|_| Vec::new(),
            &cancel,
        )
        .await
        .expect("循环应成功");
    (sink.snapshot(), transcript)
}

#[tokio::test]
async fn event_order_and_parallel_completion_order_match_the_spec() {
    let turns = vec![
        assistant(
            StopReason::ToolUse,
            vec![tool_call("call_a", "slow"), tool_call("call_b", "fast")],
        ),
        assistant(
            StopReason::Stop,
            vec![Content::Text {
                text: "done".to_string(),
                text_signature: None,
            }],
        ),
    ];
    let (events, transcript) = run(
        turns,
        vec![stub("slow", 60, "slow-out"), stub("fast", 5, "fast-out")],
        CancellationToken::new(),
    )
    .await;

    let kinds = kinds(&events);
    // tool_execution_start 按**源码顺序**（a 后 b）。
    let starts: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolExecutionStart { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(starts, vec!["call_a", "call_b"], "start 必须按源码顺序");

    // tool_execution_end 按**完成顺序**：快的先发。
    let ends: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolExecutionEnd { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        ends,
        vec!["call_b", "call_a"],
        "end 必须按完成顺序（快的先发）"
    );

    assert_eq!(kinds.first(), Some(&"agent_start"));
    // **`agent_settled` 不再由 `run()` 发**：它是"整次会话（含重试循环）"的收尾事件，
    // 由调用方的包装器在最后发出（`print_mode.rs` 的重试包装器、与 golden/S07 的 25 条事件一致）。
    // 因此单次 `run()` 的最后一条是 `agent_end`——这里断言的是**位置变化**，判据没有放宽。
    assert_eq!(kinds.last(), Some(&"agent_end"));
    assert!(kinds.contains(&"turn_start"));
    assert!(kinds.contains(&"message_start"));
    assert!(kinds.contains(&"message_end"));
    assert_eq!(
        kinds.iter().filter(|k| **k == "turn_start").count(),
        2,
        "两个 turn"
    );

    // 工具结果消息按**源码顺序**进 transcript。
    let tool_texts: Vec<&str> = transcript
        .iter()
        .filter_map(|message| match message {
            Message::ToolResult {
                tool_call_id,
                content,
                ..
            } => Some((tool_call_id.as_str(), content)),
            _ => None,
        })
        .map(|(id, content)| {
            assert!(
                matches!(content.first(), Some(ToolResultContent::Text { .. })),
                "{id} 应是文本结果"
            );
            id
        })
        .collect();
    assert_eq!(tool_texts, vec!["call_a", "call_b"], "结果消息按源码顺序");
}

#[tokio::test]
async fn length_stop_reason_skips_every_tool_call_and_reports_the_limit_error() {
    let turns = vec![assistant(
        StopReason::Length,
        vec![tool_call("call_x", "fast"), tool_call("call_y", "fast")],
    )];
    let (events, transcript) = run(
        turns,
        vec![stub("fast", 1, "should-not-run")],
        CancellationToken::new(),
    )
    .await;

    let starts = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::ToolExecutionStart { .. }))
        .count();
    // **`tool_execution_start` 仍然要发**：golden/S13b-toolcall 的事件流里
    // `tool_execution_start` 与 `tool_execution_end` 成对出现（30 条事件里有这一对）。
    // "不执行"体现在**结果**上（长度超限的错误），而不是"不发 start"。
    // 我原先断言 0 条，那是按"不执行 ⇒ 不发事件"的想当然写的，被 golden 否掉。
    assert_eq!(starts, 2, "两个工具调用都要发 start 事件");

    let texts: Vec<String> = transcript
        .iter()
        .filter_map(|message| match message {
            Message::ToolResult {
                content, is_error, ..
            } => {
                assert!(is_error, "length 的结果必须是错误");
                match content.first() {
                    Some(ToolResultContent::Text { text, .. }) => Some(text.clone()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 2);
    assert!(
        texts[0].starts_with("Tool call \"fast\" was not executed:"),
        "实际：{}",
        texts[0]
    );
}

#[tokio::test]
async fn abort_before_the_turn_makes_no_request_and_runs_no_tools() {
    // **这条测试的前提变了**（原为 `abort_truncates_calls_that_have_not_started`）：
    // 我原先假定"回合开始前就已取消"时仍然会发请求、工具批次照跑（于是产出被截断的中止结果）。
    // golden/S09b（abort 发生在工具执行中）与 ref 的流函数语义表明：
    // **带着已中止的信号去发请求时，请求不会发出去**，该回合直接物化一条错误消息
    // （`stopReason: "error"` + `errorMessage: "This operation was aborted"`，
    // 见 `crates/pi-core/src/agent/run.rs` 的 `stream_turn` 开头），
    // 于是这一轮**没有任何工具被执行**。
    //
    // 覆盖变化说明（不是放宽判据）：
    // - "未开始的调用被截断"的**纯逻辑**由 `t3_24_loop_state.rs:153,161`
    //   （`aborted_call_outcomes`）单测覆盖；
    // - 本测试现在钉住的是**循环层**的新行为：预取消 → 不发请求、不执行工具。
    let turns = vec![assistant(
        StopReason::ToolUse,
        vec![tool_call("call_a", "fast")],
    )];
    let cancel = CancellationToken::new();
    cancel.cancel();
    let (events, transcript) = run(turns, vec![stub("fast", 1, "never")], cancel).await;

    assert!(
        !transcript
            .iter()
            .any(|message| matches!(message, Message::ToolResult { .. })),
        "预取消的回合不应产出工具结果消息，实际：{transcript:#?}"
    );
    // 该回合的助手消息是**物化的错误消息**（golden/S09b 第 18/19 条：
    // `stopReason: "error"` + `errorMessage: "This operation was aborted"`）。
    let error_message = transcript
        .iter()
        .find_map(|message| match message {
            Message::Assistant(assistant) => Some(assistant),
            Message::User { .. }
            | Message::System { .. }
            | Message::ToolResult { .. }
            | Message::Custom { .. }
            | Message::BranchSummary { .. }
            | Message::CompactionSummary { .. } => None,
        })
        .expect("有助手消息");
    assert_eq!(error_message.stop_reason, StopReason::Error);
    assert_eq!(
        error_message.error_message.as_deref(),
        Some("This operation was aborted")
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolExecutionStart { .. })),
        "预取消的回合不应有工具执行事件"
    );
}
