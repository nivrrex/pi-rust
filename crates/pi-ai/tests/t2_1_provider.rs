//! T2.1：Provider trait 与错误类型。
//!
//! 判据：trait 可由 mock 实现驱动；错误类型能把 spec 里的几类失败区分开，
//! 且 `is_retryable` 的口径与 `spec/b-agent-loop.md` 的判定顺序一致。

use futures::StreamExt;
use pi_ai::{Provider, ProviderError, ProviderRequest, Stream};
use std::sync::{Arc, Mutex};

/// 单条 user 文本的上下文。
fn ctx_of(prompt: &str, system: Option<&str>) -> pi_ai::Context {
    pi_ai::Context::from_messages(
        system.map(str::to_string),
        &[pi_ai::Message::User {
            content: pi_ai::UserContent::Text(prompt.to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        }],
    )
}

/// 记录调用次数的最小 mock provider，用来验证 trait 的可驱动性。
struct CountingProvider {
    events: Vec<pi_ai::AssistantEvent>,
    calls: Arc<Mutex<u32>>,
}

impl Provider for CountingProvider {
    fn stream<'a>(
        &'a self,
        _request: ProviderRequest,
    ) -> pi_ai::BoxFuture<'a, Result<Stream, ProviderError>> {
        let events = self.events.clone();
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            *calls.lock().expect("mock 无并发写") += 1;
            Ok(Stream::from_events(events))
        })
    }
}

#[tokio::test]
async fn mock_provider_can_be_driven_through_the_trait() {
    let calls = Arc::new(Mutex::new(0));
    let provider = CountingProvider {
        events: vec![
            pi_ai::AssistantEvent::Start,
            pi_ai::AssistantEvent::TextEnd {
                content_index: 0,
                content: "hi".to_string(),
            },
        ],
        calls: Arc::clone(&calls),
    };

    let stream = provider
        .stream(ProviderRequest {
            model: "mock-model".to_string(),
            context: ctx_of("hello", None),
            max_completion_tokens: 4096,
            tools: Vec::new(),
            thinking: None,
            headers: Vec::new(),
        })
        .await
        .expect("mock 不会失败");

    let collected: Vec<pi_ai::AssistantEvent> = StreamExt::collect(stream).await;
    assert_eq!(collected.len(), 2);
    assert_eq!(*calls.lock().expect("mock 无并发写"), 1);
}

#[test]
fn retry_classification_follows_spec_order() {
    // 不可重试的限额模式要先判：即使文案里带 429 也判 false。
    let limit = ProviderError::stream_error("429 rate limit: quota exceeded / insufficient_quota");
    assert!(!limit.is_retryable(), "限额类错误不得重试");

    let throttled = ProviderError::stream_error("429 too many requests");
    assert!(throttled.is_retryable(), "瞬时节流应重试");

    let upstream = ProviderError::stream_error("Provider returned error (524)");
    assert!(upstream.is_retryable(), "上游瞬时错误应重试");

    assert!(!ProviderError::aborted().is_retryable(), "取消不得重试");
    assert!(
        !ProviderError::invalid_request("bad model").is_retryable(),
        "请求非法不得重试"
    );
}

#[test]
fn error_display_keeps_provider_text() {
    let err = ProviderError::stream_error("stream ended without message_stop");
    let text = err.to_string();
    assert!(
        text.contains("stream ended without message_stop"),
        "错误文本须保留原文：{text}"
    );
}

/// 模式表逐字对齐 `retry.ts` 后，两类 provider 文案的判定。
#[test]
fn provider_patterns_match_the_typescript_tables() {
    // 非重试的限额类：即使文案里带 429 也不重试。
    for text in [
        "GoUsageLimitError",
        "insufficient_quota",
        "Your credit balance is too low",
        "monthly usage limit reached",
    ] {
        assert!(
            !ProviderError::stream_error(text).is_retryable(),
            "限额类不应重试：{text}"
        );
    }
    // 可重试的瞬时类。
    for text in [
        "overloaded_error",
        "Rate limit exceeded",
        "429 Too Many Requests",
        "503 Service Unavailable",
        "Provider returned error (524)",
        "fetch failed",
        "socket hang up",
        "stream ended without message_stop",
        "you can retry your request",
    ] {
        assert!(
            ProviderError::stream_error(text).is_retryable(),
            "瞬时类应重试：{text}"
        );
    }
}
