//! Provider 抽象与错误类型，对应 `ref/pi-0.87.1/packages/ai/src/types.ts` 的
//! `Provider` / `StreamOptions` 与 `utils/retry.ts` 的错误分类（见 `spec/a-types.md`、
//! `spec/b-agent-loop.md`）。
//!
//! T2.1 只定义边界：trait、请求参数、事件流容器与错误类型。HTTP 传输在 T2.3 落地，
//! 重试策略在 T2.2 落地，二者都只依赖本文件的类型。
//!
//! 线程安全：trait 要求 `Send + Sync`，实现内部若有共享可变状态须自行同步。

use futures::stream::BoxStream;
use thiserror::Error;

use crate::AssistantEvent;

/// trait 方法返回的装箱 future，使 trait 保持 dyn 兼容（稳定的 `async fn in trait` 不是 dyn 兼容的）。
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// 一次请求所需的最小参数集。
///
/// 完整请求体（system/tools/messages 等）在 T2.3 按 provider 的 `api` 展开，
/// 这里只保留 provider 无关的选择项，对应 `SimpleStreamOptions` 的子集。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderRequest {
    pub model: String,
    /// 消息上下文。请求体里的 messages / system 全部由它派生，
    /// 不再由本结构单独携带 `prompt` 字符串。
    pub context: crate::context::Context,
    /// `max_completion_tokens`，对应 TS 请求体的同名字段。
    pub max_completion_tokens: u64,
    /// 本次请求可用的工具，顺序即请求体里的顺序。
    pub tools: Vec<crate::openai::ToolSpec>,
    /// 思考等级。仅 `anthropic-messages` 使用；`None` 表示不发 `thinking` 字段。
    pub thinking: Option<crate::anthropic::ThinkingConfig>,
    /// 模型自带头（对应 TS 的 `model.headers`），按插入顺序逐条下发。
    pub headers: Vec<(String, String)>,
}

/// provider 产出的助手事件流。
///
/// 不实现 `Clone`：内部是 `Pin<Box<dyn Stream>>`，克隆没有意义。需要复放时用
/// [`Stream::from_events`] 重建。
pub struct Stream {
    inner: BoxStream<'static, AssistantEvent>,
    /// 流结束时写入的最终助手消息。
    ///
    /// `Stream` 只出事件；最终消息由解码器在收尾时给出，通过这个共享槽位带出来，
    /// 免得调用方从事件里"拼"消息（那样会丢 usage 等字段）。
    final_message: std::sync::Arc<std::sync::Mutex<Option<crate::AssistantMessage>>>,
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream").finish_non_exhaustive()
    }
}

/// `Stream::final_message` 的失败：最终消息槽位的锁中毒（P4-15 步骤 5.1）。
///
/// 中毒意味着**持有该槽位的线程 panic 了**（provider 侧状态已损坏）。
/// 这不是"还没产生终态"，调用方不得把它当成正常状态。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("最终消息槽位锁中毒（持有者 panic）：无法区分\"还没产生终态\"与\"状态损坏\"")]
pub struct FinalMessagePoisoned;

impl Stream {
    /// 由任意事件流构造（不带最终消息），供测试与离线重放使用。
    pub fn new(inner: BoxStream<'static, AssistantEvent>) -> Self {
        Self {
            inner,
            final_message: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// 由事件流与最终消息槽位构造，供 HTTP 客户端使用。
    pub fn with_final_message(
        inner: BoxStream<'static, AssistantEvent>,
        final_message: std::sync::Arc<std::sync::Mutex<Option<crate::AssistantMessage>>>,
    ) -> Self {
        Self {
            inner,
            final_message,
        }
    }

    /// 流走完后取最终助手消息；还没走完时为 `Ok(None)`。
    ///
    /// **P4-15 步骤 5.1 改判**：此前用 `.lock().ok()` 把**锁中毒**也变成 `None`，
    /// 调用方无法区分"还没产生终态"与"槽位损坏（provider 侧持锁 panic）"。
    /// 现在中毒走 [`FinalMessagePoisoned`] 返回，调用方必须显式处理。
    pub fn final_message(&self) -> Result<Option<crate::AssistantMessage>, FinalMessagePoisoned> {
        let guard = self
            .final_message
            .lock()
            .map_err(|_not_guard| FinalMessagePoisoned)?;
        Ok(guard.clone())
    }

    /// 由已知事件序列构造，供测试与离线重放使用。
    pub fn from_events(events: Vec<AssistantEvent>) -> Self {
        Self::new(Box::pin(futures::stream::iter(events)))
    }

    /// 逐个取出事件。
    pub async fn next_event(&mut self) -> Option<AssistantEvent> {
        futures::StreamExt::next(&mut self.inner).await
    }
}

impl futures::Stream for Stream {
    type Item = AssistantEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.inner).poll_next(cx)
    }
}

/// provider 层的失败类型。
///
/// 变体按"能否重试"划分，与 `isRetryableAssistantError` 的判定顺序对应：
/// 先看是否命中不可重试的限额模式，再看是否命中可重试的瞬时模式。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderError {
    /// 上游返回了错误响应或流中途失败，原文保留用于模式匹配。
    #[error("provider stream error: {0}")]
    StreamError(String),

    /// 请求本身不合法（模型名、参数等），重试没有意义。
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// 传输层失败：连接被拒、DNS、超时等。
    #[error("transport error: {0}")]
    Transport(String),

    /// 调用方取消。重试必须跳过。
    #[error("request aborted")]
    Aborted,

    /// 上下文溢出：由上层决定是否压缩后重试，本层不重试。
    #[error("context overflow: {0}")]
    ContextOverflow(String),

    /// HTTP 非 2xx 响应。保留状态码、原因短语、响应体与 `Retry-After` 头。
    ///
    /// 单独成变体而不是并入流错误，是因为退避时长要取自**响应头**
    /// （`provider-retry.ts:51-65`），不能只靠响应体里的文本。
    #[error("{message}")]
    Http {
        status: u16,
        reason: String,
        body: String,
        retry_after: Option<RetryAfter>,
        /// 与 OpenAI SDK **逐字一致**的文案（见 [`openai_error_message`]）。
        ///
        /// 为什么单独存一份而不是在 `Display` 里现算：`golden/S07` 的
        /// `auto_retry_start.errorMessage` 记的就是 SDK 的原文，而这条文本还会进
        /// `session.jsonl`（P4 的 A 层）——必须在**构造时**就定型，避免各处措辞不一。
        message: String,
    },
}

/// OpenAI SDK 的错误文案（`APIError.makeMessage` / `APIError.generate`）。
///
/// **出处**：`pi-ts-0.87.1/node_modules/@earendil-works/pi-coding-agent/dist/bundle/chunks/chunk-NUHFSC37.js`
/// （该 chunk 内 `static generate(status,errorResponse,message,headers)` 与
/// `static makeMessage(status,error,message)` 两处；SDK 被打包进 bundle，故按 chunk 引用）。
///
/// 规则逐条照抄：
/// 1. `generate` 取的是 **`errorResponse?.error`**，不是整个响应体；
/// 2. `makeMessage`：`error?.message` 存在 → 是字符串就用它，否则 `JSON.stringify` 它；
///    否则 `error` 为真 → `JSON.stringify(error)`；否则用 SDK 自己的 `message` 参数（我们这里没有）；
/// 3. 有 status 且有 msg → `"{status} {msg}"`；有 status 无 msg → `"{status} status code (no body)"`。
///
/// 这解释了 golden/S07 的 `"429 status code (no body)"`：mock 的 429 体是 `{}`，
/// `body.error` 为 `undefined`，于是落到第 3 条的后半支。
pub fn openai_error_message(status: u16, body: &str) -> String {
    // 逐行对应 SDK：
    // - `openai/src/client.ts:860-861`（anthropic 侧 `@anthropic-ai/sdk/src/client.ts:1266-1267`）
    //   `const errJSON = safeJSON(errText); const errMessage = errJSON ? undefined : errText;`
    // - `openai/src/internal/utils/values.ts:99-105`（anthropic 侧同文件同名）：`safeJSON` = 解析失败返回 undefined
    // - `openai/src/core/error.ts:39-58` `makeMessage(status, error, message)`
    //
    // 注意两处 JS 真值语义：`errJSON` **为假**（`null`/`0`/`""`/`false`）时也走 `errText` 分支；
    // `error ? JSON.stringify(error)` 同理。
    let parsed = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .filter(js_truthy);
    let error = parsed
        .as_ref()
        .and_then(|value| value.get("error"))
        .filter(|value| js_truthy(value));
    // 解析失败（或解析出假值）时，**原文**就是 `message`——此前这里把它丢成 "(no body)"。
    let fallback = if parsed.is_none() { Some(body) } else { None };
    let msg: Option<String> = match error {
        Some(serde_json::Value::Object(map)) => {
            match map.get("message").filter(|value| js_truthy(value)) {
                Some(serde_json::Value::String(text)) => Some(text.clone()),
                Some(other) => Some(other.to_string()),
                None => Some(serde_json::Value::Object(map.clone()).to_string()),
            }
        }
        Some(other) => Some(other.to_string()),
        None => fallback.map(str::to_string),
    };
    match msg {
        Some(msg) if !msg.is_empty() => format!("{status} {msg}"),
        Some(_) | None => format!("{status} status code (no body)"),
    }
}

/// JS 的真值判断（`JSON` 层）：`null`/`false`/`0`/`NaN`/`""` 为假，其余为真。
fn js_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(flag) => *flag,
        serde_json::Value::Number(number) => number.as_f64().is_none_or(|float: f64| float != 0.0),
        serde_json::Value::String(text) => !text.is_empty(),
        serde_json::Value::Array(_not_scalar) => true,
        serde_json::Value::Object(_not_scalar) => true,
    }
}

/// 模式表编译失败时的错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("provider 错误模式无法编译: {0}")]
pub struct PatternError(pub String);

/// 把 TS 的模式片段拼成与 `buildProviderErrorPattern` 等价的正则：`new RegExp(parts.join("|"), "i")`。
///
/// 模式表是编译期常量，逐个编译并缓存。
fn joined_pattern(source: &'static str) -> Option<regex::Regex> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<&'static str, Option<regex::Regex>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = guard.get(source) {
        return hit.clone();
    }
    // TS 侧是 `new RegExp(parts.join("|"), "i")`；这里逐片段加 `(?i)`，
    // 对匹配结果等价，但避免整串加前缀影响锚点语义。
    let compiled = regex::Regex::new(&format!("(?i){source}")).ok();
    guard.insert(source, compiled.clone());
    compiled
}

/// `Retry-After` 头的两种形态（`provider-retry.ts:58-61`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryAfter {
    /// 秒数。
    Seconds(u64),
    /// HTTP-date 原文，解析在用到时做。
    HttpDate(String),
}

impl RetryAfter {
    /// 解析 `Retry-After` 头的值；数字按秒数，其余按 HTTP-date。
    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(seconds) = trimmed.parse::<f64>() {
            if seconds >= 0.0 {
                return Some(Self::Seconds(seconds as u64));
            }
            return None;
        }
        // TS 侧此时会走 `Date.parse()`，解析失败得到 NaN 并继续参与运算。
        // 这里选择直接判为缺失——NaN 退避没有意义，且会把非法值带进 sleep。
        if trimmed.contains(", ") && trimmed.ends_with("GMT") {
            return Some(Self::HttpDate(trimmed.to_string()));
        }
        None
    }

    /// 换算成相对 `now` 的毫秒数；HTTP-date 早于当下时返回 `None`（不等待）。
    pub fn to_millis(&self, now: std::time::SystemTime) -> Option<u64> {
        match self {
            Self::Seconds(seconds) => Some(seconds.saturating_mul(1000)),
            Self::HttpDate(raw) => http_date_to_millis(raw)
                .and_then(|target| target.duration_since(now).ok())
                .map(|d| d.as_millis() as u64),
        }
    }
}

/// 解析 HTTP-date（RFC 1123：`Sun, 06 Nov 1994 08:49:37 GMT`）。
fn http_date_to_millis(raw: &str) -> Option<std::time::SystemTime> {
    let body = raw.split_once(", ")?.1.trim();
    let mut parts = body.split(' ');
    let day: u32 = parts.next()?.parse().ok()?;
    let month = parts.next()?;
    let year: i64 = parts.next()?.parse().ok()?;
    let time = parts.next()?;
    let mut clock = time.split(':');
    let hour: u64 = clock.next()?.parse().ok()?;
    let minute: u64 = clock.next()?.parse().ok()?;
    let second: u64 = clock.next()?.parse().ok()?;
    let month_index = MONTHS.iter().position(|m| *m == month)? as i64 + 1;

    // 天数 → 民用日期（Howard Hinnant 的 civil_from_days）。
    let (y, m) = if month_index <= 2 {
        (year - 1, month_index + 12)
    } else {
        (year, month_index)
    };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + (hour * 3600 + minute * 60 + second) as i64;
    u64::try_from(secs)
        .ok()
        .map(std::time::Duration::from_secs)
        .and_then(|d| std::time::UNIX_EPOCH.checked_add(d))
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl ProviderError {
    /// 以流错误构造，保留 provider 原文。
    pub fn stream_error(message: impl Into<String>) -> Self {
        Self::StreamError(message.into())
    }

    /// 构造 HTTP 错误。
    pub fn http(
        status: u16,
        reason: impl Into<String>,
        body: impl Into<String>,
        retry_after: Option<RetryAfter>,
    ) -> Self {
        let body = body.into();
        Self::Http {
            status,
            reason: reason.into(),
            message: openai_error_message(status, &body),
            body,
            retry_after,
        }
    }

    /// 构造取消错误。
    pub fn aborted() -> Self {
        Self::Aborted
    }

    /// 把 `reqwest::Error` 连同**原因链**拼成一行（TLS/证书/DNS 的诊断信息都在 `source()` 里，
    /// 只取 `Display` 会得到笼统的 "error sending request for url …"）。
    #[must_use]
    pub fn describe_transport_error(error: &reqwest::Error) -> String {
        let mut text = error.to_string();
        let mut source: Option<&(dyn std::error::Error + 'static)> =
            std::error::Error::source(error);
        let mut depth = 0;
        while let Some(cause) = source {
            if depth >= 4 {
                break;
            }
            let cause_text = cause.to_string();
            if !cause_text.is_empty() && !text.contains(&cause_text) {
                text.push_str(": ");
                text.push_str(&cause_text);
            }
            source = cause.source();
            depth += 1;
        }
        text
    }

    /// 以传输层失败构造。
    pub fn transport(message: impl Into<String>) -> Self {
        Self::Transport(message.into())
    }

    /// 构造请求非法错误。
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    /// 订阅/额度上限类文案：命中即**不可**重试。
    /// `NON_RETRYABLE_PROVIDER_LIMIT_ERROR_PATTERN`，逐字来自 `retry.ts:7-24`。
    /// 传输层里**重试无用**的一类：TLS/证书/配置问题。放在限额模式之后、可重试之前判。
    /// 证书/TLS **配置**错误：重试不会变好。
    ///
    /// 注意**不能**写通用的 `tls`：`client error (Connect): tls handshake eof` 是
    /// `ifdown eth0` 这类瞬时断网的真实文案（真机已复现），必须可重试。
    const NON_RETRYABLE_TLS_PATTERNS: &[&'static str] = &[
        r#"certificate"#,
        r#"invalid peer"#,
        r#"unknown issuer"#,
        r#"self.signed"#,
        r#"unsupported.?protocol"#,
        r#"no.?cipher"#,
    ];

    const NON_RETRYABLE_LIMIT_PATTERNS: &[&'static str] = &[
        r#"GoUsageLimitError"#,
        r#"FreeUsageLimitError"#,
        r#"Monthly usage limit reached"#,
        r#"available balance"#,
        r#"insufficient_quota"#,
        r#"out of budget"#,
        r#"quota exceeded"#,
        r#"billing"#,
    ];

    /// 瞬时故障文案：命中则可重试。
    /// `RETRYABLE_PROVIDER_ERROR_PATTERN`，逐字来自 `retry.ts:26-88`。
    const RETRYABLE_PATTERNS: &[&'static str] = &[
        r#"overloaded"#,
        r#"currently experiencing high demand"#,
        r#"rate.?limit"#,
        r#"too many requests"#,
        r#"429"#,
        r#"500"#,
        r#"502"#,
        r#"503"#,
        r#"504"#,
        r#"520"#,
        r#"524"#,
        r#"service.?unavailable"#,
        r#"server.?error"#,
        r#"internal.?error"#,
        r#"provider.?returned.?error"#,
        r#"exceeded request buffer limit while retrying upstream"#,
        r#"network.?error"#,
        r#"connection.?error"#,
        r#"connection.?refused"#,
        r#"connection.?lost"#,
        r#"other side closed"#,
        r#"fetch failed"#,
        r#"getaddrinfo"#,
        // 网络层真实文案（reqwest/rustls/OS）：DNS、重置、中断、管道断开。
        // 真机 `ifdown eth0` 一类故障落在这些字符串上（R1 分类表）。
        r#"failed to lookup address"#,
        r#"dns error"#,
        r#"connection reset"#,
        r#"reset by peer"#,
        r#"broken pipe"#,
        r#"unexpected eof"#,
        // TLS 握手阶段被网络中断（真实文案：`client error (Connect): tls handshake eof`）。
        r#"handshake eof"#,
        r#"handshake failure"#,
        r#"tls handshake"#,
        r#"connect error"#,
        r#"client error \(connect\)"#,
        r#"connection closed before message completed"#,
        r#"connection aborted"#,
        r#"network is unreachable"#,
        r#"no route to host"#,
        r#"host unreachable"#,
        r#"ENOTFOUND"#,
        r#"EAI_AGAIN"#,
        r#"upstream.?connect"#,
        r#"reset before headers"#,
        r#"socket hang up"#,
        r#"socket connection was closed"#,
        r#"timed? out"#,
        r#"timeout"#,
        r#"terminated"#,
        r#"websocket.?closed"#,
        r#"websocket.?error"#,
        r#"ended without"#,
        r#"stream ended before message_stop"#,
        r#"stream ended before a terminal response event"#,
        r#"http2 request did not get a response"#,
        r#"retry delay"#,
        r#"you can retry your request"#,
        r#"try your request again"#,
        r#"please retry your request"#,
        r#"ResourceExhausted"#,
    ];

    /// 按**消息文本**判断可重试性（`retry.ts:237-242` 的 `isRetryableAssistantError`）。
    ///
    /// 与 [`ProviderError::is_retryable`] 的区别：产品层的重试拿到的是一条
    /// `stopReason: "error"` 的 assistant 消息，只有一个错误字符串，没有 HTTP 状态码；
    /// 因此这里**只能**按文本分类（`retry.ts` 里也是纯文本匹配）。
    /// 前提同 TS：`stopReason` 必须是 `error` 且 `errorMessage` 非空。
    pub fn is_retryable_message_text(
        stop_reason: &crate::StopReason,
        error_message: Option<&str>,
    ) -> bool {
        if !matches!(stop_reason, crate::StopReason::Error) {
            return false;
        }
        let Some(error_message) = error_message.filter(|text| !text.trim().is_empty()) else {
            return false;
        };
        let text = error_message.to_lowercase();
        if Self::matches_any(&text, Self::NON_RETRYABLE_LIMIT_PATTERNS) {
            return false;
        }
        // 证书/TLS 配置错误：重试不会变好（ref 语义同"不可重试"）。
        if Self::matches_any(&text, Self::NON_RETRYABLE_TLS_PATTERNS) {
            return false;
        }
        Self::matches_any(&text, Self::RETRYABLE_PATTERNS)
    }

    /// 判断该错误是否值得重试，口径与 `isRetryableAssistantError` 一致：
    /// 先测不可重试的限额模式，再测可重试的瞬时模式；其余变体一律不重试。
    pub fn is_retryable(&self) -> bool {
        let text = match self {
            Self::StreamError(message) => message.to_lowercase(),
            Self::Transport(message) => message.to_lowercase(),
            Self::Http { status, body, .. } => {
                // 5xx 与 429 属于瞬时/可退避；4xx 其余是请求本身的问题，不重试。
                // 仍要过一遍文本：403 的文案可能是额度上限，那属于不可重试。
                let transient = *status == 429 || (500..600).contains(status);
                let text = format!("{status} {body}").to_lowercase();
                if Self::matches_any(&text, Self::NON_RETRYABLE_LIMIT_PATTERNS) {
                    return false;
                }
                if transient {
                    return true;
                }
                text
            }
            // 请求非法、取消、上下文溢出都不重试。
            Self::InvalidRequest(_) | Self::Aborted | Self::ContextOverflow(_) => return false,
        };
        if Self::matches_any(&text, Self::NON_RETRYABLE_LIMIT_PATTERNS) {
            return false;
        }
        Self::matches_any(&text, Self::RETRYABLE_PATTERNS)
    }

    /// 模式匹配。`retry.ts` 用 `new RegExp(patterns.join("|"), "i")`，
    /// 这里对每个片段单独匹配，其中 `.` 保持"任意字符"语义。
    fn matches_any(text: &str, patterns: &[&'static str]) -> bool {
        patterns
            .iter()
            .filter_map(|source| joined_pattern(source))
            .any(|re| re.is_match(text))
    }

    /// HTTP 状态码；非 HTTP 错误返回 `None`。
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            Self::StreamError(_)
            | Self::InvalidRequest(_)
            | Self::Transport(_)
            | Self::Aborted
            | Self::ContextOverflow(_) => None,
        }
    }

    /// 该错误要求的退避时长（毫秒），来自 `retry-after` 类响应头。
    /// 没有服务端指示时返回 `None`，由退避计算走指数序列。
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            // 响应头优先于响应体文本（`provider-retry.ts:52-61`）。
            Self::Http {
                retry_after, body, ..
            } => retry_after
                .as_ref()
                .and_then(|parsed| parsed.to_millis(std::time::SystemTime::now()))
                .or_else(|| parse_retry_after(body)),
            Self::StreamError(message) => parse_retry_after(message),
            // 只有流错误与 HTTP 错误会携带服务端给出的等待时长。
            Self::InvalidRequest(_)
            | Self::Aborted
            | Self::ContextOverflow(_)
            | Self::Transport(_) => None,
        }
    }
}

/// 从错误文本里取 `retry-after` 毫秒数。
///
/// 只处理显式的 `retry-after-ms=<n>` 形式；HTTP-date 与 `retry-after` 秒数形式
/// 依赖响应头，落在 T2.3 的 HTTP 边界层。
fn parse_retry_after(message: &str) -> Option<u64> {
    let marker = "retry-after-ms=";
    let start = message.to_lowercase().find(marker)? + marker.len();
    let rest = &message[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// provider 抽象：把一次请求变成一条助手事件流。
///
/// 实现须是 `Send + Sync`；返回的流不要求线程安全，但跨 `.await` 共享时由调用方负责。
pub trait Provider: Send + Sync {
    /// 发起一次请求。
    ///
    /// 只负责建立流；错误以 [`ProviderError`] 返回，**不做重试**——重试策略在 T2.2。
    fn stream<'a>(
        &'a self,
        request: ProviderRequest,
    ) -> BoxFuture<'a, Result<Stream, ProviderError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_patterns_win_over_retryable_ones() {
        let err = ProviderError::stream_error("503 billing: rate limit exceeded");
        assert!(!err.is_retryable());
    }

    #[test]
    fn dot_in_pattern_matches_any_character() {
        assert!(ProviderError::stream_error("Rate limit reached").is_retryable());
        assert!(
            ProviderError::stream_error("Rate_Limit reached").is_retryable(),
            "`.` 也匹配下划线"
        );
        assert!(!ProviderError::stream_error("no matching words here").is_retryable());
    }

    /// 传输层文案分类（P8d/R1）：真实 reqwest/OS 原文要能判对。
    #[test]
    fn transport_errors_are_classified() {
        // 可重试：DNS / 超时 / 重置 / 中断 / 路由不可达
        for text in [
            "error sending request: dns error: failed to lookup address information: Name or service not known",
            "error sending request: connection refused",
            "operation timed out",
            "connection timed out",
            "connection reset by peer",
            "Connection closed before message completed",
            "broken pipe",
            "Network is unreachable",
            "Stream ended without finish_reason",
            // 真机 `ifdown eth0` 实测文案（reqwest + rustls 连接阶段）：
            "transport error: error sending request for url (https://openrouter.ai/api/v1/chat/completions): client error (Connect): tls handshake eof",
            "tls handshake failure",
            // 握手失败同理：多为网络/中间设备瞬断，重试有意义（证书类另有判据）。
            "tls handshake failed",
        ] {
            assert!(
                ProviderError::transport(text).is_retryable(),
                "应当可重试：{text}"
            );
        }
        // 不可重试：证书 / TLS / 限额
        for text in [
            "invalid peer certificate: UnknownIssuer",
            "certificate verify failed",
            "insufficient credits",
        ] {
            assert!(
                !ProviderError::transport(text).is_retryable(),
                "不应重试：{text}"
            );
        }
    }

    #[test]
    fn retry_after_ms_is_parsed_from_stream_error() {
        let err = ProviderError::stream_error("slow down, retry-after-ms=1500 (overloaded)");
        assert_eq!(err.retry_after_ms(), Some(1500));
        assert!(ProviderError::stream_error("overloaded")
            .retry_after_ms()
            .is_none());
    }
}

#[cfg(test)]
mod error_message_tests {
    use super::openai_error_message;

    #[test]
    fn body_without_error_field_falls_back_to_no_body() {
        // golden/S07 的真实输入：mock 的 429 体是 `{}`。
        assert_eq!(openai_error_message(429, "{}"), "429 status code (no body)");
        assert_eq!(openai_error_message(429, ""), "429 status code (no body)");
        // 非 JSON 正文**不再**退化成 "(no body)" —— 见 `non_json_body_is_used_as_the_message`
        // （SDK 把原文当 message 传进去，这是 A1 查出的不一致）。
    }

    #[test]
    fn error_message_string_is_used_verbatim() {
        assert_eq!(
            openai_error_message(400, r#"{"error":{"message":"boom"}}"#),
            "400 boom"
        );
    }

    #[test]
    fn non_string_error_message_is_stringified() {
        assert_eq!(
            openai_error_message(400, r#"{"error":{"message":{"code":"x"}}}"#),
            r#"400 {"code":"x"}"#
        );
    }

    #[test]
    fn error_object_without_message_is_stringified_whole() {
        assert_eq!(
            openai_error_message(403, r#"{"error":{"type":"forbidden"}}"#),
            r#"403 {"type":"forbidden"}"#
        );
    }

    #[test]
    fn empty_message_string_stringifies_the_whole_error_object() {
        // SDK `openai/src/core/error.ts:41-46`：
        // `error?.message ? (typeof error.message === "string" ? error.message : JSON.stringify(error.message))
        //  : error ? JSON.stringify(error) : message`
        // —— `""` 为假 → 落到 `error ? JSON.stringify(error)` = `{"message":""}`。
        assert_eq!(
            openai_error_message(400, r#"{"error":{"message":""}}"#),
            r#"400 {"message":""}"#
        );
    }

    #[test]
    fn non_json_body_is_used_as_the_message() {
        // SDK `openai/src/client.ts:860-861`（anthropic 侧同形：`@anthropic-ai/sdk/src/client.ts:1266-1267`）：
        // `const errJSON = safeJSON(errText); const errMessage = errJSON ? undefined : errText;`
        // → 解析失败时**原文进 message**（pi-rs 此前把它丢成 "(no body)"，是静默丢弃）。
        assert_eq!(openai_error_message(500, "<html>"), "500 <html>");
        // 解析成功但整个值**为假**（`null`）时同样走 errText 分支（`safeJSON("null")` 返回 `null`，JS 里为假）。
        assert_eq!(openai_error_message(400, "null"), "400 null");
        // 空白体：errText 是空串 → 假 → "no body"。
        assert_eq!(openai_error_message(400, "   "), "400    ");
    }
}
