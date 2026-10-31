//! agent loop 对外发出的事件。
//!
//! 成员与字段照 `packages/agent/src/types.ts:485-500`；一个 `turn` 的定义是
//! "一次 assistant 响应加上其后的任何工具调用与结果"（`types.ts:489-491`）。

use pi_ai::Message;
use serde_json::Value;

/// 循环过程中发出的 11 个事件。
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// 一次 run 的开始。
    AgentStart,
    /// 一次 run 的最后一个事件；`messages` 是本 run 新增的消息。
    AgentEnd {
        messages: Vec<Message>,
        /// 本次 run 是否会再重试（`agent_end` 的 `willRetry` 字段，golden/S01 实测为 `false`）。
        will_retry: bool,
    },
    /// 会话追加了一条 entry（`entry` 是该 entry 的 JSON）。golden/S07 的载荷是 `{entry: {…}}`。
    EntryAppended { entry: Value },
    /// 自动压缩开始（ref `agent-session.ts:2769` 的 `{ type: "compaction_start", reason }`）。
    CompactionStart { reason: String },
    /// 自动压缩结束（ref `agent-session.ts:2866` 的 `{ type: "compaction_end", reason, result,
    /// aborted, willRetry }`；失败路径 `:2882-2888` 另带 `errorMessage`，成功时为 `None`）。
    CompactionEnd {
        reason: String,
        /// 成功时是 `CompactionResult` 的 JSON；失败/中止时缺省（golden 的成功路径有它）。
        result: Option<Value>,
        aborted: bool,
        will_retry: bool,
        error_message: Option<String>,
    },
    /// 摘要请求将被重试（ref `agent-session.ts:191-197`；形状照 golden/S13c）。
    SummarizationRetryScheduled {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        error_message: String,
    },
    /// 摘要请求的一次重试**开始**（ref `:198-203`）。`source` 为 `"compaction"` 时带 `reason`。
    SummarizationRetryAttemptStart {
        source: String,
        reason: Option<String>,
    },
    /// 摘要请求的重试循环**结束**（成功或最终失败都发；ref `:204`，无字段）。
    SummarizationRetryFinished,
    /// 收尾（`agent-session.ts:171` 的 `{ type: "agent_settled" }`）。
    /// 它是**最后一个**事件：`agent_end` 的监听器跑完之后才发。
    AgentSettled,
    /// 一个 turn 的开始（`prepareNextTurn` 之后）。
    TurnStart,
    /// 一个 turn 的结束，带本轮 assistant 消息与工具结果消息。
    /// 队列内容变化（入队/消费/清空）。ref `agent-session.ts:171-177` 的 `queue_update`。
    ///
    /// **不带载荷**：剩余内容由出口侧（持有队列的实现）读取后填，
    /// 循环本身不需要知道队列里还剩什么。
    QueueUpdate,

    TurnEnd {
        message: Message,
        tool_results: Vec<Message>,
    },
    /// 一条消息开始（流式时为 assistant 的初始快照）。
    MessageStart { message: Message },
    /// assistant 消息的流式增量；**只对流式中的 assistant 消息**发出
    /// （`types.ts:494-495`）。
    ///
    /// `assistant_event` 是 provider 层的原始增量事件（`text_delta` 等）。json 模式的
    /// 线格式要把它投影出来（`spec/b-agent-loop.md` 第 3 节），所以必须带在事件里，
    /// 不能只留累积快照。
    MessageUpdate {
        message: Message,
        assistant_event: Option<pi_ai::AssistantEvent>,
    },
    /// 一条消息定稿（权威内容）。
    MessageEnd { message: Message },
    /// 一个工具调用开始执行。
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    /// 执行中的进度（流式输出时用）。
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        /// 与 `ToolExecutionStart` 相同的入参（golden/S05 实测每条 update 都重复带它）。
        args: Value,
        partial: Value,
    },
    /// 产品层准备重试：`{ attempt, maxAttempts, delayMs, errorMessage }`。
    /// `errorMessage` 为空时填 `"Unknown error"`（`agent-session.ts:189-190` 一带的规则）。
    AutoRetryStart {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        error_message: String,
    },
    /// 重试的收尾：`{ success, attempt, finalError? }`。
    AutoRetryEnd {
        success: bool,
        attempt: u32,
        final_error: Option<String>,
    },
    /// 一个工具调用结束。**按完成顺序发出**（`agent-loop.ts:616-625`）。
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: Value,
        is_error: bool,
    },
}

impl AgentEvent {
    /// 线格式里的 `type` 字段。
    pub fn kind(&self) -> &'static str {
        match self {
            AgentEvent::AgentStart => "agent_start",
            AgentEvent::AgentEnd { .. } => "agent_end",
            AgentEvent::EntryAppended { .. } => "entry_appended",
            AgentEvent::CompactionStart { .. } => "compaction_start",
            AgentEvent::CompactionEnd { .. } => "compaction_end",
            AgentEvent::SummarizationRetryScheduled { .. } => "summarization_retry_scheduled",
            AgentEvent::SummarizationRetryAttemptStart { .. } => {
                "summarization_retry_attempt_start"
            }
            AgentEvent::SummarizationRetryFinished => "summarization_retry_finished",
            AgentEvent::AgentSettled => "agent_settled",
            AgentEvent::TurnStart => "turn_start",
            AgentEvent::TurnEnd { .. } => "turn_end",
            AgentEvent::QueueUpdate => "queue_update",
            AgentEvent::MessageStart { .. } => "message_start",
            AgentEvent::MessageUpdate { .. } => "message_update",
            AgentEvent::MessageEnd { .. } => "message_end",
            AgentEvent::ToolExecutionStart { .. } => "tool_execution_start",
            AgentEvent::ToolExecutionUpdate { .. } => "tool_execution_update",
            AgentEvent::ToolExecutionEnd { .. } => "tool_execution_end",
            AgentEvent::AutoRetryStart { .. } => "auto_retry_start",
            AgentEvent::AutoRetryEnd { .. } => "auto_retry_end",
        }
    }
}
