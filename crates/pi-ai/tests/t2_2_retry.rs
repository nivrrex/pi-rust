//! T2.2：重试策略。
//!
//! 判据对齐 `ref/pi-0.87.1/packages/ai/src/utils/provider-retry.ts`：
//! `maxRetries` 默认 0、服务端 `retry-after` 优先于指数退避、超上限**抛错而非截断**、
//! 退避可被取消。

use std::sync::{Arc, Mutex};

use pi_ai::retry::{RetryPolicy, RetrySleeper, DEFAULT_MAX_RETRY_DELAY_MS};
use pi_ai::ProviderError;
use tokio_util::sync::CancellationToken;

/// 记录每次请求与退避时长的假 sleeper，避免测试真的等待。
#[derive(Default)]
struct RecordingSleeper {
    delays_ms: Arc<Mutex<Vec<u64>>>,
    cancelled: Arc<Mutex<u32>>,
    /// 第一次等待时顺手取消 token，用来模拟"退避途中被取消"。
    cancel_on_sleep: bool,
}

impl RetrySleeper for RecordingSleeper {
    fn sleep_ms<'a>(
        &'a self,
        ms: u64,
        token: &'a CancellationToken,
    ) -> pi_ai::retry::BoxRetryFuture<'a, ()> {
        let delays = Arc::clone(&self.delays_ms);
        let cancelled = Arc::clone(&self.cancelled);
        Box::pin(async move {
            delays.lock().expect("假 sleeper 无并发写").push(ms);
            if self.cancel_on_sleep {
                token.cancel();
            }
            if token.is_cancelled() {
                *cancelled.lock().expect("假 sleeper 无并发写") += 1;
                return;
            }
            tokio::select! {
                _ = token.cancelled() => {
                    *cancelled.lock().expect("假 sleeper 无并发写") += 1;
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(0)) => {}
            }
        })
    }
}

fn policy(sleeper: &RecordingSleeper, max_retries: u32) -> RetryPolicy {
    policy_with(sleeper, max_retries, false)
}

fn policy_with(sleeper: &RecordingSleeper, max_retries: u32, cancel_on_sleep: bool) -> RetryPolicy {
    RetryPolicy::new(max_retries).with_sleeper(Arc::new(RecordingSleeper {
        delays_ms: Arc::clone(&sleeper.delays_ms),
        cancelled: Arc::clone(&sleeper.cancelled),
        cancel_on_sleep,
    }))
}

#[tokio::test]
async fn default_policy_does_not_retry() {
    let sleeper = RecordingSleeper::default();
    let calls = Arc::new(Mutex::new(0));
    let seen = Arc::clone(&calls);

    let result = policy(&sleeper, 0)
        .run(
            {
                let seen = Arc::clone(&calls);
                move || {
                    let seen = Arc::clone(&seen);
                    async move {
                        *seen.lock().expect("无并发写") += 1;
                        Err::<(), _>(ProviderError::stream_error("429 too many requests"))
                    }
                }
            },
            &CancellationToken::new(),
        )
        .await;

    assert_eq!(*calls.lock().expect("无并发写"), 1, "默认不重试");
    assert!(result.is_err());
    assert!(
        sleeper.delays_ms.lock().expect("无并发写").is_empty(),
        "默认不重试就没有退避"
    );
    drop(seen);
}

#[tokio::test]
async fn retryable_error_is_retried_until_success() {
    let sleeper = RecordingSleeper::default();
    let calls = Arc::new(Mutex::new(0));

    let value = policy(&sleeper, 3)
        .run(
            {
                let calls = Arc::clone(&calls);
                move || {
                    let calls = Arc::clone(&calls);
                    async move {
                        let mut n = calls.lock().expect("无并发写");
                        *n += 1;
                        if *n < 3 {
                            Err(ProviderError::stream_error("503 service unavailable"))
                        } else {
                            Ok(*n)
                        }
                    }
                }
            },
            &CancellationToken::new(),
        )
        .await
        .expect("第三次应成功");

    assert_eq!(value, 3);
    assert_eq!(
        sleeper.delays_ms.lock().expect("无并发写").len(),
        2,
        "两次失败各退避一次"
    );
}

#[tokio::test]
async fn non_retryable_error_is_not_retried() {
    let sleeper = RecordingSleeper::default();
    let calls = Arc::new(Mutex::new(0));

    let result = policy(&sleeper, 5)
        .run(
            {
                let calls = Arc::clone(&calls);
                move || {
                    let calls = Arc::clone(&calls);
                    async move {
                        *calls.lock().expect("无并发写") += 1;
                        Err::<(), _>(ProviderError::stream_error("insufficient_quota"))
                    }
                }
            },
            &CancellationToken::new(),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(*calls.lock().expect("无并发写"), 1, "限额类错误不重试");
}

#[tokio::test]
async fn server_retry_after_wins_over_exponential_backoff() {
    let sleeper = RecordingSleeper::default();
    let calls = Arc::new(Mutex::new(0));

    let _ = policy(&sleeper, 1)
        .run(
            {
                let calls = Arc::clone(&calls);
                move || {
                    let calls = Arc::clone(&calls);
                    async move {
                        let mut n = calls.lock().expect("无并发写");
                        *n += 1;
                        Err::<(), _>(ProviderError::stream_error(
                            "overloaded, retry-after-ms=1500",
                        ))
                    }
                }
            },
            &CancellationToken::new(),
        )
        .await;

    assert_eq!(
        sleeper.delays_ms.lock().expect("无并发写").as_slice(),
        &[1500],
        "服务端指定的退避应原样使用"
    );
}

#[tokio::test]
async fn retry_delay_above_limit_raises_instead_of_truncating() {
    let sleeper = RecordingSleeper::default();
    let calls = Arc::new(Mutex::new(0));

    let result = policy(&sleeper, 1)
        .run(
            {
                let calls = Arc::clone(&calls);
                move || {
                    let calls = Arc::clone(&calls);
                    async move {
                        let mut n = calls.lock().expect("无并发写");
                        *n += 1;
                        Err::<(), _>(ProviderError::stream_error(
                            "overloaded, retry-after-ms=90000",
                        ))
                    }
                }
            },
            &CancellationToken::new(),
        )
        .await;

    let err = result.expect_err("超上限应抛错");
    assert!(
        err.to_string().contains("retry delay"),
        "错误文案应含 retry delay，实际：{err}"
    );
    assert_eq!(*calls.lock().expect("无并发写"), 1, "超上限时不再重试");
}

#[test]
fn default_max_delay_is_sixty_seconds() {
    assert_eq!(DEFAULT_MAX_RETRY_DELAY_MS, 60_000);
}

#[tokio::test]
async fn cancellation_during_backoff_stops_the_loop() {
    let sleeper = RecordingSleeper::default();
    let token = CancellationToken::new();
    token.cancel();
    let calls = Arc::new(Mutex::new(0));

    let result = policy(&sleeper, 5)
        .run(
            {
                let calls = Arc::clone(&calls);
                move || {
                    let calls = Arc::clone(&calls);
                    async move {
                        *calls.lock().expect("无并发写") += 1;
                        Err::<(), _>(ProviderError::stream_error("503 overloaded"))
                    }
                }
            },
            &token,
        )
        .await;

    assert!(
        matches!(result, Err(ProviderError::Aborted)),
        "取消后应返回 Aborted"
    );
    assert_eq!(
        *calls.lock().expect("无并发写"),
        1,
        "与 TS 一致：先发一次请求再检查 abort"
    );
    assert!(
        sleeper.delays_ms.lock().expect("无并发写").is_empty(),
        "已取消时不再进入退避"
    );
}

#[tokio::test]
async fn cancelling_during_backoff_stops_further_attempts() {
    let sleeper = RecordingSleeper::default();
    let token = CancellationToken::new();
    let calls = Arc::new(Mutex::new(0));

    let result = policy_with(&sleeper, 5, true)
        .run(
            {
                let calls = Arc::clone(&calls);
                move || {
                    let calls = Arc::clone(&calls);
                    async move {
                        *calls.lock().expect("无并发写") += 1;
                        Err::<(), _>(ProviderError::stream_error("503 overloaded"))
                    }
                }
            },
            &token,
        )
        .await;

    assert!(
        matches!(result, Err(ProviderError::Aborted)),
        "退避途中被取消应返回 Aborted"
    );
    assert_eq!(*calls.lock().expect("无并发写"), 1, "退避被取消后不再重试");
    assert_eq!(sleeper.delays_ms.lock().expect("无并发写").len(), 1);
    assert_eq!(*sleeper.cancelled.lock().expect("无并发写"), 1);
}
