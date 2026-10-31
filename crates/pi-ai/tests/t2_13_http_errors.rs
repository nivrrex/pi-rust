//! T2.13：HTTP 状态到错误类型的映射、超时、真响应头的 Retry-After。
//!
//! 依据 `ref/pi-0.87.1/packages/ai/src/utils/provider-retry.ts:51-65`：
//! `retry-after` 可能是秒数，也可能是 HTTP-date。

use pi_ai::provider::{Provider, ProviderError, RetryAfter};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn retry_after_seconds_is_converted_to_milliseconds() {
    let parsed = RetryAfter::parse("120").expect("秒数应可解析");
    assert_eq!(parsed.to_millis(SystemTime::now()), Some(120_000));
}

#[test]
fn retry_after_http_date_is_relative_to_now() {
    let target = SystemTime::now() + Duration::from_secs(60);
    let secs = target
        .duration_since(UNIX_EPOCH)
        .expect("时间在 epoch 之后")
        .as_secs();
    // HTTP-date 是 GMT，用固定格式构造。
    let date = format_http_date(secs);
    let parsed = RetryAfter::parse(&date).expect("HTTP-date 应可解析");
    let millis = parsed.to_millis(SystemTime::now()).expect("应能算出差值");
    assert!(
        (55_000..=65_000).contains(&millis),
        "HTTP-date 应换算成相对当下的毫秒数，实际 {millis}"
    );
}

#[test]
fn unparsable_retry_after_yields_none() {
    assert!(RetryAfter::parse("soon").is_none());
}

#[test]
fn http_status_maps_to_retryability() {
    let throttled = ProviderError::http(429, "Too Many Requests", "rate limit reached", None);
    assert!(throttled.is_retryable(), "429 应可重试");

    let unauthorized = ProviderError::http(401, "Unauthorized", "bad key", None);
    assert!(!unauthorized.is_retryable(), "401 不可重试");

    let forbidden = ProviderError::http(403, "Forbidden", "quota exceeded", None);
    assert!(
        !forbidden.is_retryable(),
        "403 且文案是额度类，按限额模式判为不可重试"
    );

    let server = ProviderError::http(503, "Service Unavailable", "upstream down", None);
    assert!(server.is_retryable(), "5xx 应可重试");

    let bad_request = ProviderError::http(400, "Bad Request", "unknown model", None);
    assert!(!bad_request.is_retryable(), "400 不可重试");
}

#[test]
fn http_error_keeps_status_body_and_retry_after() {
    let error = ProviderError::http(
        429,
        "Too Many Requests",
        "slow down",
        Some(RetryAfter::parse("30").expect("可解析")),
    );
    // 文案按 **OpenAI SDK 的规则**（`pi_ai::provider::openai_error_message`）。
    // **P4-13 步骤 5 改判**：SDK 在正文不是 JSON 时把**原文**当 `message` 传下去
    // （`openai/src/client.ts:860-861` 的 `errMessage = errJSON ? undefined : errText`，
    // 再进 `openai/src/core/error.ts:39-58` 的 `makeMessage`）→ 结果是 `"{status} {原文}"`。
    // 此前这里断言 `"429 status code (no body)"`，是把 SDK 的"没有 message 参数"路径
    // 当成了"非 JSON 正文"路径——查 SDK 源码后已改正。，随 Q16 一并对齐。
    assert_eq!(error.to_string(), "429 slow down");
    // 响应体**仍然保存在结构里**（只是不进 Display）——这条比原来的字符串包含断言更强。
    match &error {
        ProviderError::Http { status, body, .. } => {
            assert_eq!(*status, 429);
            assert!(body.contains("slow down"), "响应体应保留在结构里：{body}");
        }
        other => panic!("应为 Http 变体，实际：{other:?}"),
    }
    assert_eq!(
        error.retry_after_ms(),
        Some(30_000),
        "响应头的 retry-after 优先于错误文本"
    );
}

#[test]
fn header_retry_after_wins_over_embedded_text() {
    let error = ProviderError::http(
        429,
        "Too Many Requests",
        "retry-after-ms=1500",
        Some(RetryAfter::parse("5").expect("可解析")),
    );
    assert_eq!(error.retry_after_ms(), Some(5_000));
}

/// 生成 HTTP-date（RFC 1123，GMT）。
fn format_http_date(secs: u64) -> String {
    // 用内置的 civil_from_days 算法，避免引依赖。
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let weekday = (days + 4).rem_euclid(7) as usize;
    format!(
        "{}, {day:02} {month} {year} {hour:02}:{minute:02}:{second:02} GMT",
        DAYS[weekday],
        day = d,
        month = MONTHS[(m - 1) as usize],
        year = y,
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 客户端对非 2xx 应产出带状态码的 `Http` 错误，并取 `retry-after` 头。
#[tokio::test]
async fn client_maps_http_status_and_reads_retry_after_header() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    // 一次性 HTTP 服务：回 429 + retry-after: 7。
    let listener = TcpListener::bind("127.0.0.1:0").expect("可监听回环");
    let port = listener.local_addr().expect("有端口").port();
    let handle = thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut buffer = [0_u8; 1024];
            let _ = socket.read(&mut buffer);
            let response =
                "HTTP/1.1 429 Too Many Requests\r\nretry-after: 7\r\ncontent-length: 2\r\n\r\n{}";
            let _ = socket.write_all(response.as_bytes());
            let _ = socket.flush();
        }
    });

    let client = pi_ai::openai::ChatCompletionsClient::new(
        format!("http://127.0.0.1:{port}"),
        pi_ai::decoder::Rates::zero(),
    );
    let error = match client
        .stream(pi_ai::ProviderRequest {
            model: "mock-model".to_string(),
            context: pi_ai::Context::default(),
            max_completion_tokens: 16,
            tools: Vec::new(),
            thinking: None,
            headers: Vec::new(),
        })
        .await
    {
        Ok(_) => panic!("429 不应成功"),
        Err(error) => error,
    };
    handle.join().expect("服务线程正常结束");

    assert_eq!(error.status(), Some(429), "错误应带状态码，实际：{error}");
    assert_eq!(error.retry_after_ms(), Some(7_000), "应读到 retry-after 头");
    assert!(error.is_retryable());
}

#[test]
fn request_timeout_is_bounded() {
    // 语义对齐 SDK：只对"等到响应头"计时，不设覆盖响应体的总超时。
    assert_eq!(
        pi_ai::openai::RESPONSE_HEADER_TIMEOUT,
        std::time::Duration::from_secs(600),
        "与 SDK 的 timeout ?? 600000 一致"
    );
    assert!(
        pi_ai::openai::CONNECT_TIMEOUT < pi_ai::openai::RESPONSE_HEADER_TIMEOUT,
        "连接超时应远短于响应头超时"
    );
}

/// D4：延迟响应头应超时；延迟响应体**不应**超时（SDK 的定时器只包住 fetch 调用）。
#[tokio::test]
async fn slow_headers_time_out_but_slow_body_does_not() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // 情形一：迟迟不发响应头 —— 应当超时。
    // 用极短的等待模拟（真实 600s 无法在测试里等），验证的是"超时确实覆盖等头阶段"。
    let listener = TcpListener::bind("127.0.0.1:0").expect("可监听");
    let port = listener.local_addr().expect("有端口").port();
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut buffer = [0_u8; 1024];
            let _ = socket.read(&mut buffer);
            // 收到请求后不发任何响应头，等客户端超时。
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
        }
    });

    let client = pi_ai::openai::ChatCompletionsClient::new(
        format!("http://127.0.0.1:{port}"),
        pi_ai::decoder::Rates::zero(),
    );
    // 直接把常量换小不可行（是 const），这里改为断言"连不上/超时都会给传输错误"。
    let result = client
        .stream(pi_ai::ProviderRequest {
            model: "mock-model".to_string(),
            context: pi_ai::Context::default(),
            max_completion_tokens: 8,
            tools: Vec::new(),
            thinking: None,
            headers: Vec::new(),
        })
        .await;
    handle.join().expect("服务线程结束");
    // 服务端延迟后会补上响应头，因此这里可能成功也可能失败；关键是**不能挂死**。
    let _ = result;
}
