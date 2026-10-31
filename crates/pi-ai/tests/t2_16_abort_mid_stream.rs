//! B4 / S09：流式中途取消，保留已收到的部分、停止原因为 aborted。
//!
//! 判据：`golden/S09/session.jsonl`（`stopReason: "aborted"`、`errorMessage: "Request aborted"`）
//! 与 `provider-retry.ts:69-71` 的 `createAbortError`。

use pi_ai::decoder::{Rates, StreamDecoder, OPENAI_COMPLETIONS};
use pi_ai::{AssistantEvent, StopReason};
use tokio_util::sync::CancellationToken;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn first_events(count: usize) -> Vec<u8> {
    let bytes =
        std::fs::read(workspace().join("fixtures/sse/openai/text-basic.sse")).expect("夹具存在");
    let text = String::from_utf8(bytes).expect("夹具是 UTF-8");
    let mut out = String::new();
    for event in text.split("\n\n").take(count) {
        out.push_str(event);
        out.push_str("\n\n");
    }
    out.into_bytes()
}

#[test]
fn abort_before_any_content_keeps_empty_content_and_aborted_reason() {
    let token = CancellationToken::new();
    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法")
        .with_cancel(token.clone());

    let before = decoder.push(&first_events(1)).expect("取消前不应报错");
    assert!(!before.is_empty(), "取消前应有事件");
    token.cancel();

    assert!(
        decoder.push(b"data: {}\n\n").is_err(),
        "取消后 push 应报错而不是继续产出"
    );
    assert!(decoder.finish().is_ok(), "finish 不应报错");

    let message = decoder.final_message();
    assert_eq!(message.stop_reason, StopReason::Aborted);
    assert_eq!(
        message.error_message.as_deref(),
        Some("Request aborted"),
        "错误文案与 TS 的 createAbortError 一致"
    );
    assert!(message.content.is_empty(), "没有收到内容时不应凭空造块");
}

#[test]
fn abort_mid_text_keeps_the_received_prefix() {
    let token = CancellationToken::new();
    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法")
        .with_cancel(token.clone());

    // 前两个事件：role 与第一个文本增量。
    let _ = decoder.push(&first_events(2)).expect("取消前不应报错");
    token.cancel();
    let _ = decoder.push(&first_events(3)).expect_err("取消后应报错");
    let _ = decoder.finish();

    let message = decoder.final_message();
    assert_eq!(message.stop_reason, StopReason::Aborted);
    let text: String = message
        .content
        .iter()
        .filter_map(|block| match block {
            pi_ai::Content::Text { text, .. } => Some(text.clone()),
            pi_ai::Content::Thinking { .. }
            | pi_ai::Content::ToolCall { .. }
            | pi_ai::Content::Image { .. } => None,
        })
        .collect();
    assert!(
        !text.is_empty(),
        "已收到的部分文本要保留，实际 {:?}",
        message.content
    );
}

#[test]
fn abort_does_not_emit_done_or_error_events() {
    let token = CancellationToken::new();
    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法")
        .with_cancel(token.clone());
    let _ = decoder.push(&first_events(1)).expect("取消前不应报错");
    token.cancel();
    let mut seen = Vec::new();
    if let Ok(events) = decoder.finish() {
        for event in events {
            seen.push(event.type_name());
        }
    }
    assert!(
        !seen.contains(&"done") && !seen.contains(&"error"),
        "取消后不应再发终止事件，实际 {seen:?}"
    );
    let _ = AssistantEvent::Start;
}
