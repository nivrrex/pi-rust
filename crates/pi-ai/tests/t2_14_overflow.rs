//! B4 / S13c：context overflow 识别（纯函数）。
//!
//! 模式表与判定顺序逐字照 `ref/pi-0.87.1/packages/ai/src/utils/overflow.ts`：
//! 先测 `NON_OVERFLOW_PATTERNS` 排除限流等误报，再测 `OVERFLOW_PATTERNS`；
//! 另有 cerebras 无正文模式，以及静默/长度两种 usage 情形。

use pi_ai::overflow::{is_context_overflow, is_recoverable_length};
use pi_ai::{StopReason, Usage};

fn usage(input: u64, output: u64) -> Usage {
    Usage {
        input,
        output,
        cache_read: 0,
        cache_write: 0,
        cache_write_1h: None,
        reasoning: None,
        total_tokens: input + output,
        cost: pi_ai::Cost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
            total: 0.0,
        },
    }
}

fn errored(text: &str, provider: &str) -> pi_ai::AssistantMessage {
    pi_ai::AssistantMessage {
        role: "assistant".to_string(),
        content: Vec::new(),
        api: "openai-completions".to_string(),
        provider: provider.to_string(),
        model: "mock-model".to_string(),
        response_model: None,
        usage: usage(0, 0),
        stop_reason: StopReason::Error,
        error_message: Some(text.to_string()),
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 0,
    }
}

#[test]
fn recognises_provider_overflow_messages() {
    let cases = [
        "prompt is too long: 213462 tokens > 200000 maximum",
        "413 {\"error\":{\"type\":\"request_too_large\",\"message\":\"Request exceeds the maximum size\"}}",
        "Your input exceeds the context window of this model",
        "Requested token count exceeds the model's maximum context length of 131072 tokens",
        "Input length (265330) exceeds model's maximum context length (262144).",
        "The input token count (1196265) exceeds the maximum number of tokens allowed (1048575)",
        "This model's maximum prompt length is 131072 but the request contains 537812 tokens",
        "Please reduce the length of the messages or completion",
        "This endpoint's maximum context length is 128000 tokens. However, you requested about 300000 tokens",
        "Input length 300000 exceeds the maximum allowed input length of 262144 tokens.",
        "The input (300000 tokens) is longer than the model's context length (262144 tokens).",
        "prompt token count of 300000 exceeds the limit of 262144",
        "the request exceeds the available context size, try increasing it",
        "tokens to keep from the initial prompt is greater than the context length",
        "invalid params, context window exceeds limit",
        "Your request exceeded model token limit: 300000 (requested: 262144)",
        "too large for model with 262144 maximum context length",
        "Prompt has 300000 tokens, but the configured context size is 262144 tokens",
        "model_context_window_exceeded",
        "prompt too long; exceeded max context length",
        "Range of input length should be [1, 262144]",
        "context_length_exceeded",
        "too many tokens",
        "token limit exceeded",
    ];
    for case in cases {
        assert!(
            is_context_overflow(&errored(case, "p"), None),
            "应识别为 overflow：{case}"
        );
    }
}

#[test]
fn rate_limit_errors_are_excluded_even_when_they_look_like_overflow() {
    // Bedrock 的节流文案含 "too many tokens"，若不排除会被误判。
    let throttled = errored(
        "Throttling error: Too many tokens, please wait before trying again.",
        "bedrock",
    );
    assert!(
        !is_context_overflow(&throttled, None),
        "Bedrock 节流不是 overflow"
    );

    let rate_limited = errored("rate limit exceeded for too many tokens", "p");
    assert!(
        !is_context_overflow(&rate_limited, None),
        "限流不是 overflow"
    );

    let unavailable = errored("Service unavailable: context window is exceeded", "p");
    assert!(
        !is_context_overflow(&unavailable, None),
        "服务不可用不是 overflow"
    );

    let too_many = errored("429 Too Many Requests", "p");
    assert!(!is_context_overflow(&too_many, None), "429 不是 overflow");
}

#[test]
fn cerebras_bodyless_overflow_is_recognised_only_for_cerebras() {
    let message = errored("400 status code (no body)", "cerebras");
    assert!(
        is_context_overflow(&message, None),
        "cerebras 的无正文 4xx 是 overflow"
    );
    let other = errored("400 status code (no body)", "p");
    assert!(
        !is_context_overflow(&other, None),
        "其他 provider 的无正文 4xx 不算"
    );
}

#[test]
fn silent_overflow_is_detected_from_usage() {
    let mut message = errored("", "p");
    message.stop_reason = StopReason::Stop;
    message.error_message = None;
    message.usage = usage(300_000, 10);
    assert!(
        is_context_overflow(&message, Some(200_000)),
        "成功但 input 超窗 = 静默溢出"
    );

    message.usage = usage(150_000, 10);
    assert!(
        !is_context_overflow(&message, Some(200_000)),
        "未超窗不算溢出"
    );
}

#[test]
fn length_stop_with_zero_output_filling_the_window_is_overflow() {
    let mut message = errored("", "p");
    message.stop_reason = StopReason::Length;
    message.error_message = None;
    message.usage = usage(199_000, 0);
    assert!(
        is_context_overflow(&message, Some(200_000)),
        "length + output 0 + input 逼近窗口 = 长度截断溢出"
    );

    message.usage = usage(199_000, 50);
    assert!(
        !is_context_overflow(&message, Some(200_000)),
        "有输出就不是这一类"
    );
}

#[test]
fn non_error_stop_reason_never_counts_as_overflow() {
    let mut message = errored("prompt is too long", "p");
    message.stop_reason = StopReason::Stop;
    message.usage = usage(0, 0);
    assert!(
        !is_context_overflow(&message, Some(200_000)),
        "stopReason 不是 error 时只看 usage 分支"
    );
}

#[test]
fn recoverable_length_requires_a_below_limit_output() {
    let mut message = errored("", "p");
    message.stop_reason = StopReason::Length;
    message.usage = usage(1000, 500);

    assert!(
        is_recoverable_length(&message, 4096),
        "输出低于期望上限可恢复"
    );
    assert!(!is_recoverable_length(&message, 0), "期望上限为 0 时不判定");
    message.usage = usage(1000, 4096);
    assert!(
        !is_recoverable_length(&message, 4096),
        "输出达到上限说明是被截断而非溢出"
    );
}
