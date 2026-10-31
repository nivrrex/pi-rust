//! agent loop 的**纯状态机**：只做调度决策，不碰 I/O、不持全局状态。
//!
//! 分工（P3.4 要求"纯状态机部分与 I/O 部分分离"）：
//! - 本模块：给定"这一轮发生了什么"（停止原因、是否还有工具调用、两个队列里各有多少
//!   消息、`finishTurn` 的决定），算出**下一步动作**与事件的先后顺序；
//! - I/O 侧（后续步骤）：真正发请求、跑工具、发事件。
//!
//! 所有规则都带 `文件:行号` 出处，对应 `spec/b-agent-loop.md`。
//!
//! **并发**：本模块全是无状态的纯函数与值类型，天然线程安全；调用方持有的运行时状态
//! （队列、上下文）由调用方自己声明所有权与锁策略。

pub mod compaction;
pub mod compaction_prompts;
pub mod event;
pub mod projection;
pub mod queue;
pub mod run;
pub mod section;

pub use event::AgentEvent;
pub use projection::last_declared_tools;
pub use queue::{user_message, PendingQueue};
pub use run::{
    run_loop, tool_state_changes, tools_added_json, tools_added_raw, AgentLoop, CollectSink,
    EventSink, LoopConfig, LoopError, NoopSink,
};

use pi_ai::StopReason;

/// 工具批次的执行模式。〔agent-loop.ts:512-519〕
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Parallel,
    Sequential,
}

/// 一个批次走串行还是并行。
///
/// 规则：**配置要求串行**，或**本批任一工具自身要求串行**，则整批串行；否则并行。
/// 〔packages/agent/src/agent-loop.ts:512-519〕
pub fn batch_execution_mode(
    configured: ExecutionMode,
    tool_modes: &[ExecutionMode],
) -> ExecutionMode {
    if configured == ExecutionMode::Sequential || tool_modes.contains(&ExecutionMode::Sequential) {
        ExecutionMode::Sequential
    } else {
        ExecutionMode::Parallel
    }
}

/// `finishTurn` 的返回。〔packages/agent/src/types.ts:254-258〕
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FinishDecision {
    /// `{ action: "end" }`：直接结束 run。
    End,
    /// `{ action: "continue" }`：保证再来一次 provider 请求。
    Continue,
    /// `undefined`：保持常规调度。
    #[default]
    Undefined,
}

/// 一轮（`turn`）结束时循环要做的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnAction {
    /// emit `turn_end` 后 emit `agent_end` 并返回。
    EndRun,
    /// 继续内层循环（还有工具调用或 steering 消息）。
    ContinueInner,
    /// 内层退出，去拉 follow-up；若拿到消息就开新一轮内层，否则看 `explicit_continuation`。
    PollFollowUp,
}

/// 一轮的实际结果。
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub stop_reason: StopReason,
    /// assistant 消息里是否还有待执行的工具调用。
    pub has_more_tool_calls: bool,
    /// 本轮的 steering 消息条数（已拉取）。
    pub steering_count: usize,
    /// follow-up 队列里的条数。
    pub follow_up_count: usize,
    pub finish: FinishDecision,
}

/// 一轮结束后的调度决策。
///
/// 规则（`agent-loop.ts:244-313`）：
/// 1. `stopReason` 是 `error`/`aborted` → **硬退出**，不执行任何工具（`toolResults: []`）；
/// 2. `finishTurn` 返回 `end` → 硬退出；
/// 3. 否则 `explicit_continuation = (finish == continue)`；若还有工具调用或有 steering
///    消息，把 `explicit_continuation` 复位为 false 并继续内层；
/// 4. 都没有 → 退出内层去拉 follow-up。
///
/// 返回 `(动作, 复位后的 explicit_continuation)`。
pub fn after_turn(outcome: &TurnOutcome) -> (TurnAction, bool) {
    if matches!(outcome.stop_reason, StopReason::Error | StopReason::Aborted) {
        return (TurnAction::EndRun, false);
    }
    if outcome.finish == FinishDecision::End {
        return (TurnAction::EndRun, false);
    }
    let mut explicit_continuation = outcome.finish == FinishDecision::Continue;
    if outcome.has_more_tool_calls || outcome.steering_count > 0 {
        explicit_continuation = false;
        return (TurnAction::ContinueInner, explicit_continuation);
    }
    (TurnAction::PollFollowUp, explicit_continuation)
}

/// 拉完 follow-up 之后的决策：开新一轮内层，还是靠 `explicit_continuation` 再来一次，
/// 还是收工。
///
/// `explicit_continuation` 的兑现方式是"一个只有上下文、没有新消息的 turn"。
/// 〔packages/agent/src/agent-loop.ts:300-313〕
pub fn after_follow_up(follow_up_count: usize, explicit_continuation: bool) -> TurnAction {
    if follow_up_count > 0 || explicit_continuation {
        TurnAction::ContinueInner
    } else {
        TurnAction::EndRun
    }
}

/// 队列的取出模式。〔packages/agent/src/agent.ts:142-172〕
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainMode {
    /// 一次全取走。
    All,
    /// 只取队首一条（默认）。〔packages/agent/src/agent.ts:244-245〕
    OneAtATime,
}

/// 按模式决定本次取走几条。
pub fn drain_count(mode: DrainMode, queued: usize) -> usize {
    match mode {
        DrainMode::All => queued,
        DrainMode::OneAtATime => queued.min(1),
    }
}

/// `stopReason == "length"` 时，本批工具调用**一律不执行**，统一报错。
///
/// 理由（`agent-loop.ts:468-474` 的注释）：流式工具参数是尽力而为的 JSON salvage 解析的，
/// 截断的响应可能产出"能解析、能校验、但静默不完整"的参数，因此没有一个是安全的。
///
/// 返回错误文本；返回 `None` 表示不是 `length`，不需要这条特判。
pub fn length_limit_error(stop_reason: &StopReason, tool_name: &str) -> Option<String> {
    if !matches!(stop_reason, StopReason::Length) {
        return None;
    }
    Some(format!(
        "Tool call \"{tool_name}\" was not executed: the response hit the output token limit, so its arguments may be truncated. Re-issue the tool call with complete arguments."
    ))
}

/// 批次是否应当提前终止：**批次非空**且每个已 finalize 的结果都要求终止。
/// 〔packages/agent/src/agent-loop.ts:685-687〕
pub fn should_terminate_batch(results: &[bool]) -> bool {
    !results.is_empty() && results.iter().all(|terminate| *terminate)
}

/// 并行批次里"还没轮到就开始被 abort"的调用所产出的错误文本。
/// 〔packages/agent/src/agent-loop.ts:616-625〕
pub const OPERATION_ABORTED: &str = "Operation aborted";

/// 走完一个批次时，哪些调用因为 abort 而没有开始执行。
///
/// 语义：一旦 `aborted` 为真，**尚未开始**的调用不再执行，改为产出
/// `Operation aborted` 的错误结果；串行/并行两条路径都在每次循环后检查 abort 并
/// `break`，即 abort 会截断本批剩余调用。〔agent-loop.ts:610-612,616-625,638-640〕
///
/// 返回每一条调用的处置：`Some(错误文本)` 表示"未执行、报这个错"。
pub fn aborted_call_outcomes(
    total: usize,
    already_finished: usize,
    aborted: bool,
) -> Vec<Option<String>> {
    (0..total)
        .map(|index| {
            if aborted && index >= already_finished {
                Some(OPERATION_ABORTED.to_string())
            } else {
                None
            }
        })
        .collect()
}
