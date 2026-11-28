//! P3.4：agent loop 纯状态机的单测。
//!
//! 每条断言都对应 `spec/b-agent-loop.md` 的一条规则（出处见被测函数文档）。

use pi_ai::StopReason;
use pi_core::agent::{
    aborted_call_outcomes, after_follow_up, after_turn, batch_execution_mode, drain_count,
    length_limit_error, should_terminate_batch, DrainMode, ExecutionMode, FinishDecision,
    TurnAction, TurnOutcome, OPERATION_ABORTED,
};

fn outcome(
    stop_reason: StopReason,
    has_more_tool_calls: bool,
    steering_count: usize,
    follow_up_count: usize,
    finish: FinishDecision,
) -> TurnOutcome {
    TurnOutcome {
        stop_reason,
        has_more_tool_calls,
        steering_count,
        follow_up_count,
        finish,
    }
}

#[test]
fn error_and_aborted_stop_reasons_end_the_run_immediately() {
    for stop in [StopReason::Error, StopReason::Aborted] {
        let (action, explicit) = after_turn(&outcome(stop, true, 3, 3, FinishDecision::Continue));
        assert_eq!(action, TurnAction::EndRun, "{stop:?} 应硬退出");
        assert!(!explicit);
    }
}

#[test]
fn finish_turn_end_ends_the_run_even_with_pending_tools() {
    let (action, _) = after_turn(&outcome(StopReason::Stop, true, 0, 0, FinishDecision::End));
    assert_eq!(action, TurnAction::EndRun);
}

#[test]
fn pending_tool_calls_keep_the_inner_loop_going_and_reset_explicit_continuation() {
    let (action, explicit) = after_turn(&outcome(
        StopReason::ToolUse,
        true,
        0,
        0,
        FinishDecision::Continue,
    ));
    assert_eq!(action, TurnAction::ContinueInner);
    assert!(!explicit, "有工具调用时 explicit_continuation 必须复位");
}

#[test]
fn steering_messages_keep_the_inner_loop_going() {
    let (action, explicit) = after_turn(&outcome(
        StopReason::Stop,
        false,
        1,
        0,
        FinishDecision::Continue,
    ));
    assert_eq!(action, TurnAction::ContinueInner);
    assert!(!explicit);
}

#[test]
fn no_tools_and_no_steering_goes_to_follow_up_polling() {
    let (action, explicit) = after_turn(&outcome(
        StopReason::Stop,
        false,
        0,
        2,
        FinishDecision::Undefined,
    ));
    assert_eq!(action, TurnAction::PollFollowUp);
    assert!(!explicit);
}

#[test]
fn explicit_continuation_survives_when_there_is_nothing_else_to_do() {
    let (action, explicit) = after_turn(&outcome(
        StopReason::Stop,
        false,
        0,
        0,
        FinishDecision::Continue,
    ));
    assert_eq!(action, TurnAction::PollFollowUp);
    assert!(explicit, "finish=continue 的意图要留给 follow-up 之后兑现");
}

#[test]
fn follow_up_messages_start_a_new_inner_round() {
    assert_eq!(after_follow_up(1, false), TurnAction::ContinueInner);
    assert_eq!(after_follow_up(3, true), TurnAction::ContinueInner);
    assert_eq!(after_follow_up(0, false), TurnAction::EndRun);
    assert_eq!(after_follow_up(0, true), TurnAction::ContinueInner);
}

#[test]
fn batch_is_sequential_if_configured_or_any_tool_demands_it() {
    use ExecutionMode::{Parallel, Sequential};
    assert_eq!(
        batch_execution_mode(Sequential, &[Parallel, Parallel]),
        Sequential
    );
    assert_eq!(
        batch_execution_mode(Parallel, &[Parallel, Sequential]),
        Sequential
    );
    assert_eq!(
        batch_execution_mode(Parallel, &[Parallel, Parallel]),
        Parallel
    );
    assert_eq!(
        batch_execution_mode(Parallel, &[]),
        Parallel,
        "空批次默认并行"
    );
}

#[test]
fn queue_drain_mode_limits_how_many_messages_are_taken() {
    assert_eq!(drain_count(DrainMode::All, 5), 5);
    assert_eq!(drain_count(DrainMode::OneAtATime, 5), 1);
    assert_eq!(drain_count(DrainMode::OneAtATime, 0), 0);
    assert_eq!(drain_count(DrainMode::All, 0), 0);
}

#[test]
fn length_stop_reason_turns_every_tool_call_into_an_error() {
    let text = length_limit_error(&StopReason::Length, "read").expect("length 要报错");
    assert_eq!(
        text,
        "Tool call \"read\" was not executed: the response hit the output token limit, so its arguments may be truncated. Re-issue the tool call with complete arguments."
    );
    assert!(length_limit_error(&StopReason::ToolUse, "read").is_none());
    assert!(length_limit_error(&StopReason::Stop, "read").is_none());
}

#[test]
fn batch_terminates_only_when_non_empty_and_all_results_ask_for_it() {
    assert!(should_terminate_batch(&[true, true]));
    assert!(!should_terminate_batch(&[true, false]));
    assert!(!should_terminate_batch(&[]), "空批次不得触发终止");
}

#[test]
fn abort_truncates_only_the_calls_that_have_not_finished() {
    let outcomes = aborted_call_outcomes(3, 1, true);
    assert_eq!(outcomes[0], None, "已完成的调用不受影响");
    assert_eq!(outcomes[1].as_deref(), Some(OPERATION_ABORTED));
    assert_eq!(outcomes[2].as_deref(), Some(OPERATION_ABORTED));
}

#[test]
fn without_abort_every_call_runs() {
    assert!(aborted_call_outcomes(2, 0, false)
        .iter()
        .all(Option::is_none));
}

// —— steering / follow-up 队列（`agent.ts:142-172`）——

#[test]
fn queue_drains_by_mode_and_reports_its_length() {
    use pi_core::agent::{user_message, DrainMode, PendingQueue};

    let queue = PendingQueue::new();
    assert!(queue.is_empty());
    queue.push(user_message("a"));
    queue.push(user_message("b"));
    queue.push(user_message("c"));
    assert_eq!(queue.len(), 3);

    // one-at-a-time：只取队首一条。
    let first = queue.drain(DrainMode::OneAtATime);
    assert_eq!(first.len(), 1, "默认模式一次只投递一条");
    assert_eq!(
        match &first[0] {
            pi_ai::Message::User { content, .. } => match content {
                pi_ai::UserContent::Text(text) => text.clone(),
                pi_ai::UserContent::Blocks(_) => String::new(),
            },
            _ => String::new(),
        },
        "a"
    );

    // all：剩下的全取走。
    let rest = queue.drain(DrainMode::All);
    assert_eq!(rest.len(), 2);
    assert!(queue.is_empty());
    assert!(
        queue.drain(DrainMode::OneAtATime).is_empty(),
        "空队列取出为空"
    );
}
