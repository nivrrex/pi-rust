//! B4 / S13a 空响应、S13b length 截断。
//!
//! 判据：`golden/S13a`、`golden/S13b-text` 里 TS 实际写下的 assistant 消息。

use pi_ai::decoder::{Rates, StreamDecoder, OPENAI_COMPLETIONS};
use pi_ai::{AssistantEvent, StopReason};

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn decode(fixture: &str) -> (Vec<&'static str>, pi_ai::AssistantMessage) {
    let bytes =
        std::fs::read(workspace().join("fixtures/sse/openai").join(fixture)).expect("夹具存在");
    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法");
    let mut names = Vec::new();
    for event in decoder.push(&bytes).expect("解码不应报错") {
        names.push(event.type_name());
    }
    for event in decoder.finish().expect("收尾不应报错") {
        names.push(event.type_name());
    }
    (names, decoder.final_message())
}

fn text_of(message: &pi_ai::AssistantMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            pi_ai::Content::Text { text, .. } => Some(text.clone()),
            pi_ai::Content::Thinking { .. }
            | pi_ai::Content::ToolCall { .. }
            | pi_ai::Content::Image { .. } => None,
        })
        .collect()
}

#[test]
fn s13a_empty_reply_yields_no_content_block() {
    let (names, message) = decode("empty-reply.sse");
    assert_eq!(
        names,
        vec!["start", "done"],
        "空回复不应产出任何内容块，实际 {names:?}"
    );
    assert_eq!(
        message.content,
        Vec::new(),
        "golden 里 content 是空数组，不是空文本块"
    );
    assert_eq!(message.stop_reason, StopReason::Stop);
    assert_eq!(message.raw_stop_reason.as_deref(), Some("stop"));
}

#[test]
fn s13b_length_stop_keeps_partial_text_and_records_raw_reason() {
    let (names, message) = decode("length-text.sse");
    assert!(names.contains(&"text_start") && names.contains(&"text_end"));
    assert_eq!(names.last(), Some(&"done"));
    assert_eq!(message.stop_reason, StopReason::Length);
    assert_eq!(message.raw_stop_reason.as_deref(), Some("length"));
    assert_eq!(text_of(&message), "开头", "截断时保留已收到的部分文本");
    assert_eq!(
        message.usage.output, 4096,
        "golden 记录的 output（被夹具写死为上限）"
    );
}

#[test]
fn length_stop_is_not_treated_as_a_normal_stop() {
    let (_, message) = decode("length-text.sse");
    assert_ne!(message.stop_reason, StopReason::Stop);
    assert_ne!(
        message.stop_reason,
        StopReason::Error,
        "截断不是错误，上层据此决定要不要压缩重试"
    );
}

#[test]
fn done_event_carries_the_mapped_stop_reason() {
    let bytes =
        std::fs::read(workspace().join("fixtures/sse/openai/length-text.sse")).expect("夹具存在");
    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法");
    let mut done = None;
    for event in decoder.push(&bytes).expect("解码不应报错") {
        if let AssistantEvent::Done { reason } = event {
            done = Some(reason);
        }
    }
    assert_eq!(
        done,
        Some(StopReason::Length),
        "done 事件应带映射后的停止原因"
    );
}

/// 流被截断（没有 finish_reason / [DONE]）时的收尾行为，对照 golden/S08。
#[test]
fn truncated_stream_finishes_as_error_without_done_event() {
    let root = workspace();
    let bytes = std::fs::read(root.join("fixtures/sse/openai/text-basic.sse")).expect("夹具存在");
    // 截掉最后两个事件（finish_reason 与 [DONE]）。
    let text = String::from_utf8(bytes).expect("UTF-8");
    let head: String = text
        .split("\n\n")
        .take(3)
        .map(|part| format!("{part}\n\n"))
        .collect();

    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法");
    let mut names = Vec::new();
    for event in decoder.push(head.as_bytes()).expect("解码不应报错") {
        names.push(event.type_name());
    }
    for event in decoder.finish().expect("收尾不应报错") {
        names.push(event.type_name());
    }
    assert!(
        !names.contains(&"done"),
        "截断的流不应发 done，实际 {names:?}"
    );

    let message = decoder.final_message();
    assert_eq!(
        message.stop_reason,
        StopReason::Error,
        "缺 finish_reason 的收尾按错误处理"
    );
    assert_eq!(
        message.error_message.as_deref(),
        Some("Stream ended without finish_reason"),
        "错误文案与 golden/S08 一致"
    );
}
