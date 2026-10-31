//! 重试策略，对应 `ref/pi-0.87.1/packages/ai/src/utils/provider-retry.ts`
//! （见 `spec/b-agent-loop.md`）。
//!
//! 与 TS 版一致的要点：
//! - `max_retries` 默认 **0**，即默认不重试；
//! - 每次重试都是一次全新的请求，不带任何重试计数头；
//! - 退避优先用服务端给的 `retry-after`，其次指数退避并加抖动；
//! - 服务端要求的退避**超过上限时抛错**，不截断；
//! - 退避可被取消（SDK 自带的计时器不理会 abort 信号，这是 Rust 侧自己实现的原因）。
//!
//! 线程安全：`RetryPolicy` 可跨线程共享（内部只有不可变字段 + `Arc<dyn>`）。

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::ProviderError;

/// 服务端要求的单次退避上限，默认 60 秒（`DEFAULT_MAX_RETRY_DELAY_MS`）。
pub const DEFAULT_MAX_RETRY_DELAY_MS: u64 = 60_000;

/// 指数退避的基数与上限：第 `i` 次重试等 `min(0.5 * 2^i, 8)` 秒。
const BASE_DELAY_MS: u64 = 500;
const MAX_EXPONENTIAL_DELAY_MS: u64 = 8_000;

/// 退避等待用的装箱 future，保持 trait dyn 兼容。
pub type BoxRetryFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// 等待指定毫秒，并在 `token` 被取消时提前返回。
///
/// 实现必须是 `Send + Sync` 且可在多个任务间共享。
pub trait RetrySleeper: Send + Sync {
    /// 等待 `ms` 毫秒；`token` 已取消时应尽快返回。
    fn sleep_ms<'a>(&'a self, ms: u64, token: &'a CancellationToken) -> BoxRetryFuture<'a, ()>;
}

/// 默认 sleeper：用 tokio 的定时器。
#[derive(Debug, Default)]
struct TokioSleeper;

impl RetrySleeper for TokioSleeper {
    fn sleep_ms<'a>(&'a self, ms: u64, token: &'a CancellationToken) -> BoxRetryFuture<'a, ()> {
        Box::pin(async move {
            tokio::select! {
                _ = token.cancelled() => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(ms)) => {}
            }
        })
    }
}

/// 抖动系数来源。指数退避会乘 `1 - rand * 0.25`，抖动上限 25%。
pub trait Jitter: Send + Sync {
    /// 返回 `[0, 1)` 的随机数。
    fn next(&self) -> f64;
}

/// 默认抖动源：基于时间的 xorshift，够用且不引依赖。
struct TimeJitter;

impl Jitter for TimeJitter {
    fn next(&self) -> f64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        let mut x = nanos | 1;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// 退避时长超出上限时的错误。
///
/// 与 TS 版一致：这种情况**抛错**而不是截断，让上层重试策略接手
/// （错误文案含 `retry delay`，会被 `is_retryable` 判为可重试）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("server requested {requested_ms}ms retry delay (max: {max_ms}ms): {source_message}")]
pub struct RetryDelayTooLong {
    /// 服务端要求的毫秒数。
    pub requested_ms: u64,
    /// 配置的上限毫秒数。
    pub max_ms: u64,
    /// 触发该判定的错误原文。
    pub source_message: String,
}

/// 重试策略。
#[derive(Clone)]
pub struct RetryPolicy {
    /// 最多重试几次；默认 0（不重试）。
    pub max_retries: u32,
    /// 单次退避的上限毫秒数。
    pub max_retry_delay_ms: u64,
    sleeper: Arc<dyn RetrySleeper>,
    jitter: Arc<dyn Jitter>,
}

impl std::fmt::Debug for RetryPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryPolicy")
            .field("max_retries", &self.max_retries)
            .field("max_retry_delay_ms", &self.max_retry_delay_ms)
            .finish_non_exhaustive()
    }
}

impl RetryPolicy {
    /// 以 `max_retries` 构造，默认上限 60 秒、默认 sleeper 与抖动源。
    pub fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            max_retry_delay_ms: DEFAULT_MAX_RETRY_DELAY_MS,
            sleeper: Arc::new(TokioSleeper),
            jitter: Arc::new(TimeJitter),
        }
    }

    /// 替换 sleeper（测试注入）。
    #[must_use]
    pub fn with_sleeper(mut self, sleeper: Arc<dyn RetrySleeper>) -> Self {
        self.sleeper = sleeper;
        self
    }

    /// 替换抖动源（测试注入确定性序列）。
    #[must_use]
    pub fn with_jitter(mut self, jitter: Arc<dyn Jitter>) -> Self {
        self.jitter = jitter;
        self
    }

    /// 指数退避时长：`min(0.5 * 2^i, 8)` 秒，再乘 `1 - rand * 0.25` 的抖动。
    pub fn exponential_delay_ms(&self, retry_index: u32) -> u64 {
        let shift = retry_index.min(16);
        let base = BASE_DELAY_MS
            .saturating_mul(1u64 << shift)
            .min(MAX_EXPONENTIAL_DELAY_MS);
        let jitter = self.jitter.next().clamp(0.0, 1.0);
        (base as f64 * (1.0 - jitter * 0.25)).round() as u64
    }

    /// 本次重试前应等待多久。
    ///
    /// 服务端指定的 `retry-after` 优先；指定值超过上限时返回
    /// [`RetryDelayTooLong`]，交由上层处理而不是截断。
    pub fn delay_ms(
        &self,
        error: &ProviderError,
        retry_index: u32,
    ) -> Result<u64, RetryDelayTooLong> {
        match error.retry_after_ms() {
            Some(requested) if requested > self.max_retry_delay_ms => Err(RetryDelayTooLong {
                requested_ms: requested,
                max_ms: self.max_retry_delay_ms,
                source_message: error.to_string(),
            }),
            Some(requested) => Ok(requested),
            None => Ok(self.exponential_delay_ms(retry_index)),
        }
    }

    /// 执行 `request`，失败时按策略重试。
    ///
    /// 每次重试都是一次全新的调用（`request` 是工厂而非 future），
    /// 与 SDK 以 `maxRetries: 0` 调用时每次都是新请求一致。
    pub async fn run<F, Fut, T>(
        &self,
        mut request: F,
        token: &CancellationToken,
    ) -> Result<T, ProviderError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, ProviderError>>,
    {
        let mut remaining = self.max_retries;
        loop {
            // 与 TS 版一致：先发请求，只有出错后才检查 abort；
            // 因此即使调用时 token 已取消，也会先尝试一次请求。
            let error = match request().await {
                Ok(value) => return Ok(value),
                Err(error) => error,
            };
            if token.is_cancelled() {
                return Err(ProviderError::aborted());
            }
            if remaining == 0 || !error.is_retryable() {
                return Err(error);
            }
            let retry_index = self.max_retries - remaining;
            remaining -= 1;

            let delay = self.delay_ms(&error, retry_index).map_err(|too_long| {
                ProviderError::stream_error(format!(
                    "server requested {}s retry delay (max: {}s). {}",
                    too_long.requested_ms.div_ceil(1000),
                    too_long.max_ms.div_ceil(1000),
                    too_long.source_message
                ))
            })?;

            self.sleeper.sleep_ms(delay, token).await;
            if token.is_cancelled() {
                return Err(ProviderError::aborted());
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    struct FixedJitter(f64);

    impl Jitter for FixedJitter {
        fn next(&self) -> f64 {
            self.0
        }
    }

    #[test]
    fn exponential_backoff_doubles_then_caps() {
        let policy = RetryPolicy::new(8).with_jitter(Arc::new(FixedJitter(0.0)));
        let delays: Vec<u64> = (0..6).map(|i| policy.exponential_delay_ms(i)).collect();
        assert_eq!(
            delays,
            vec![500, 1000, 2000, 4000, 8000, 8000],
            "第 4 次起封顶 8 秒"
        );
    }

    #[test]
    fn jitter_only_shortens_the_delay_by_at_most_a_quarter() {
        let policy = RetryPolicy::new(8).with_jitter(Arc::new(FixedJitter(1.0)));
        assert_eq!(policy.exponential_delay_ms(0), 375, "500ms 乘 0.75");
    }

    #[test]
    fn delay_above_limit_is_reported_not_truncated() {
        let policy = RetryPolicy::new(1);
        let error = ProviderError::stream_error("overloaded, retry-after-ms=90000");
        let too_long = policy
            .delay_ms(&error, 0)
            .expect_err("超过 60 秒上限应报错");
        assert_eq!(too_long.requested_ms, 90_000);
        assert_eq!(too_long.max_ms, DEFAULT_MAX_RETRY_DELAY_MS);
    }
}
