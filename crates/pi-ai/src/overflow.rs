//! context overflow 识别，对应 `ref/pi-0.87.1/packages/ai/src/utils/overflow.ts`
//! （见 `spec/d-compaction.md`）。纯函数，无状态。
//!
//! 模式表**逐字照抄** TS；用 `regex` 编译（依赖已在 `notes/questions.md` 的 Q9 获批准）。
//! 判定顺序照 TS：先排除 `NON_OVERFLOW_PATTERNS`（限流等误报），再测 `OVERFLOW_PATTERNS`。

use std::sync::OnceLock;

use regex::Regex;

use crate::AssistantMessage;

/// `OVERFLOW_PATTERNS`，逐字来自 `overflow.ts:9-63`。
const OVERFLOW_SOURCES: &[&str] = &[
    r#"prompt (?:is )?too long"#,
    r#"request_too_large"#,
    r#"input is too long for requested model"#,
    r#"exceeds the context window"#,
    r#"exceeds (?:the )?(?:model'?s )?maximum context length(?: of [\d,]+ tokens?|\s*\([\d,]+\))"#,
    r#"input token count.*exceeds the maximum"#,
    r#"maximum prompt length is \d+"#,
    r#"reduce the length of the messages"#,
    r#"maximum context length is \d+ tokens"#,
    r#"exceeds (?:the )?maximum allowed input length of [\d,]+ tokens?"#,
    r#"input \(\d+ tokens\) is longer than the model'?s context length \(\d+ tokens\)"#,
    r#"exceeds the limit of \d+"#,
    r#"exceeds the available context size"#,
    r#"greater than the context length"#,
    r#"context window exceeds limit"#,
    r#"exceeded model token limit"#,
    r#"too large for model with \d+ maximum context length"#,
    r#"prompt has [\d,]+ tokens?, but the configured context size is [\d,]+ tokens?"#,
    r#"model_context_window_exceeded"#,
    r#"prompt too long; exceeded (?:max )?context length"#,
    r#"range of input length should be"#,
    r#"context[_ ]length[_ ]exceeded"#,
    r#"too many tokens"#,
    r#"token limit exceeded"#,
];

/// `NON_OVERFLOW_PATTERNS`，逐字来自 `overflow.ts:75-79`。
const NON_OVERFLOW_SOURCES: &[&str] = &[
    r#"^(Throttling error|Service unavailable):"#,
    r#"rate limit"#,
    r#"too many requests"#,
];

/// cerebras 的无正文 4xx，逐字来自 `overflow.ts:65`。
const CEREBRAS_BODYLESS_OVERFLOW_PATTERN: &str = r#"^4(?:00|13)\s*(?:status code)?\s*\(no body\)"#;

struct Patterns {
    overflow: Vec<Regex>,
    non_overflow: Vec<Regex>,
    cerebras: Regex,
}

/// 模式表编译失败（抄写错误）时的错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("overflow 模式表无法编译: {0}")]
pub struct PatternError(pub String);

/// TS 的每条模式都带 `/i` 标志；抽出字面量时标志会丢，这里统一补回。
///
/// 模式表是编译期固定的常量字面量，编译失败只可能是抄写错误。
/// 返回 [`PatternError`] 而不 panic——调用方应在启动时校验一次并决定怎么处理。
fn compile(source: &str) -> Result<Regex, PatternError> {
    Regex::new(&format!("(?i){source}")).map_err(|e| PatternError(format!("{source}: {e}")))
}

/// 校验模式表；启动时调一次即可。
pub fn validate_patterns() -> Result<(), PatternError> {
    match patterns() {
        Ok(_) => Ok(()),
        Err(error) => Err(error.clone()),
    }
}

fn patterns() -> &'static Result<Patterns, PatternError> {
    static CACHE: OnceLock<Result<Patterns, PatternError>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Ok(Patterns {
            overflow: OVERFLOW_SOURCES
                .iter()
                .map(|source| compile(source))
                .collect::<Result<_, _>>()?,
            non_overflow: NON_OVERFLOW_SOURCES
                .iter()
                .map(|source| compile(source))
                .collect::<Result<_, _>>()?,
            cerebras: compile(CEREBRAS_BODYLESS_OVERFLOW_PATTERN)?,
        })
    })
}

/// 判断一条助手消息是否是上下文溢出（`overflow.ts:136-170`）。
///
/// 三种情形：错误文案命中、静默溢出（成功但 input 超窗）、
/// 长度截断溢出（`length` + output 为 0 + input 逼近窗口）。
pub fn is_context_overflow(message: &AssistantMessage, context_window: Option<u64>) -> bool {
    let patterns = match patterns() {
        Ok(patterns) => patterns,
        // 模式表编译不过说明源码抄错了，此时退化为只做 usage 分支。
        Err(error) => return classify_error_on_broken_patterns(message, context_window, error),
    };
    if message.stop_reason == crate::StopReason::Error {
        if let Some(text) = &message.error_message {
            let excluded = patterns.non_overflow.iter().any(|re| re.is_match(text));
            if !excluded {
                if patterns.overflow.iter().any(|re| re.is_match(text)) {
                    return true;
                }
                if message.provider == "cerebras" && patterns.cerebras.is_match(text) {
                    return true;
                }
            }
        }
    }

    if let Some(window) = context_window {
        let input_tokens = message.usage.input + message.usage.cache_read;
        if message.stop_reason == crate::StopReason::Stop && input_tokens > window {
            return true;
        }
        // 输入被填满窗口、没有余量生成时，服务端以 `length` + output 0 返回。
        if message.stop_reason == crate::StopReason::Length
            && message.usage.output == 0
            && input_tokens as f64 >= window as f64 * 0.99
        {
            return true;
        }
    }
    false
}

/// 模式表损坏时的兜底：只做不依赖模式表的 usage 分支。
///
/// 正常路径不会走到这里；保留它是为了让"模式表抄错"不会退化成"永不溢出"。
fn classify_error_on_broken_patterns(
    message: &AssistantMessage,
    context_window: Option<u64>,
    _error: &PatternError,
) -> bool {
    // 只依赖 usage，不依赖任何模式表。
    let Some(window) = context_window else {
        return false;
    };
    let input_tokens = message.usage.input + message.usage.cache_read;
    (message.stop_reason == crate::StopReason::Stop && input_tokens > window)
        || (message.stop_reason == crate::StopReason::Length
            && message.usage.output == 0
            && input_tokens as f64 >= window as f64 * 0.99)
}

/// 判断 `length` 停止是否是因为输出没到期望上限（`overflow.ts:178-180`）。
///
/// `desired_max_output` 必须是**未被上下文夹紧**的原始上限。
pub fn is_recoverable_length(message: &AssistantMessage, desired_max_output: u64) -> bool {
    message.stop_reason == crate::StopReason::Length
        && desired_max_output > 0
        && message.usage.output < desired_max_output
}
