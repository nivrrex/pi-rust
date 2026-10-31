//! `openai-completions` 适配层的请求构造，对照 `golden/S12-openai/requests.jsonl`
//! 里 TS 实际发出的请求体（见 `spec/a-types.md` 与 `spec/f-config.md` 的 compat 字段）。

use serde_json::{json, Map, Value};

use crate::message::Message;
use crate::{Content, ProviderRequest};

/// 一个工具的声明，`parameters` 是 JSON Schema object 的子集。
///
/// 形状对照 `golden/S02/requests.jsonl`：`{name, description, parameters}`。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// 原样透传的 JSON Schema。
    pub parameters: Value,
}

impl ToolSpec {
    /// 转成 `openai-completions` 的 `{type:"function", function:{…}}` 形状。
    pub fn to_chat_completions_json(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}

/// 构造 `/v1/chat/completions` 的请求体。
///
/// 字段集合照 TS 实际发出的请求体：model、messages、stream、stream_options、
/// store、max_completion_tokens、tools。
///
/// **字段顺序不保证**：启用 `serde_json` 的 `preserve_order` 需要新增 indexmap 依赖，
/// 超出 AGENTS 允许的依赖清单；而顺序对 C 层判据无影响（比较的是解析后的 JSON），
/// 因此不引入。
///
pub fn build_chat_completions_request(request: &ProviderRequest) -> Value {
    build_chat_completions_request_with_provider(request, "")
}

/// 同 `build_chat_completions_request`，但**把 provider 作为参数传入**——
/// 发送侧的 reasoning 字段名要看它（ref `openai-completions.ts:1335-1337` 的 opencode-go 特例）。
/// 注意：**不给 `ProviderRequest` 加字段**（那会动 36 处构造点），由调用方（client 已有 `provider`）传入。
pub fn build_chat_completions_request_with_provider(
    request: &ProviderRequest,
    provider: &str,
) -> Value {
    let mut body = Map::new();

    body.insert("model".to_string(), json!(request.model));
    body.insert(
        "messages".to_string(),
        Value::Array(request.context.to_openai_messages_with_provider(provider)),
    );
    body.insert("stream".to_string(), json!(true));
    body.insert(
        "stream_options".to_string(),
        json!({ "include_usage": true }),
    );
    body.insert("store".to_string(), json!(false));
    body.insert(
        "max_completion_tokens".to_string(),
        json!(request.max_completion_tokens),
    );
    // **思考等级下发成 `reasoning_effort`**：ref `openai-completions.ts:925-928`。
    // golden/S16 实测：等级 `high` → `"reasoning_effort":"high"`。
    if let Some(effort) = &request.context.reasoning_effort {
        body.insert("reasoning_effort".to_string(), json!(effort));
    }
    // ref `openai-completions.ts:848-856` 的三分支：
    // ① 有要请求的工具 ⇒ 正常写；② 没有工具但**历史里有工具调用/结果** ⇒ 写空数组
    // （Anthropic 经代理时要求这种会话必须带 `tools`）；③ 两者都不满足 ⇒ **不写该键**。
    // golden/S19 的摘要请求正是第三种（没有工具、历史里也没有工具调用）。
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_string(),
            Value::Array(
                request
                    .tools
                    .iter()
                    .map(ToolSpec::to_chat_completions_json)
                    .collect(),
            ),
        );
    } else if has_tool_history(&request.context.messages) {
        body.insert("tools".to_string(), Value::Array(Vec::new()));
    }
    Value::Object(body)
}

use std::collections::VecDeque;

use futures::StreamExt as _;
use tokio_util::sync::CancellationToken;

use crate::decoder::{Rates, StreamDecoder, OPENAI_COMPLETIONS};

/// ref `openai-completions.ts:88-99` 的 `hasToolHistory`：消息里出现工具结果，
/// 或助手消息里出现工具调用块，即认为该会话"有工具历史"。
#[must_use]
fn has_tool_history(messages: &[Message]) -> bool {
    messages.iter().any(|message| match message {
        Message::ToolResult { .. } => true,
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .any(|block| matches!(block, Content::ToolCall { .. })),
        Message::User { .. }
        | Message::System { .. }
        | Message::Custom { .. }
        | Message::BranchSummary { .. }
        | Message::CompactionSummary { .. } => false,
    })
}

/// 等到**响应头**的超时（600 秒）。
///
/// 语义逐字对齐 SDK：`@anthropic-ai/sdk/client.js:638-676` 的 `fetchWithTimeout`
/// 把定时器只包在底层 `fetch` 调用外，`await baseFetch(...)` 返回后立即
/// `clearTimeout`。也就是说它覆盖的是"等到响应头"，**不覆盖响应体流式读取**——
/// 否则超过 600 秒的长流式输出会被截断。
///
/// 600000 这个数值来自该 SDK 的 `timeout ?? 600000`（`resources/beta/messages/messages.js:44`）。
pub const RESPONSE_HEADER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// 建立 TCP/TLS 连接的超时。
///
/// SDK 没有单独的这一项（它由 undici 的默认行为兜底）；这里显式设一个较短的连接
/// 超时，避免端点不可达时白等 600 秒。**这一项是工程判断**，不是 SDK 的规格。
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
use crate::provider::{BoxFuture, Provider, Stream};
use crate::AssistantEvent;
use crate::ProviderError;

/// `openai-completions` 的费率（每百万 token）。
///
/// 单独命名以便与 `decoder::Rates` 区分：后者是解码器内部的四元组。
pub type OpenAiRates = Rates;

/// `openai-completions` 的认证头。
///
/// **发送方已定位**：`api/openai-completions.ts:787-793` 用 `new OpenAI({apiKey, …})`
/// 构造客户端，认证头由 **OpenAI SDK** 下发（`Authorization: Bearer <key>`）；
/// pi 自己只往 `defaultHeaders` 里放 `User-Agent` 与模型自带头。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthHeader {
    /// `authorization: Bearer <key>`——OpenAI SDK 的默认形态。
    #[default]
    Bearer,
    /// `x-api-key: <key>`（Anthropic 系与部分网关）。
    ApiKey,
    /// 不加认证头（本地 mock、自带 headers 的网关）。
    None,
}

impl AuthHeader {
    /// 请求头名称；[`AuthHeader::None`] 返回空串。
    pub fn header_name(self) -> &'static str {
        match self {
            Self::Bearer => "authorization",
            Self::ApiKey => "x-api-key",
            Self::None => "",
        }
    }

    /// 请求头的取值。空 key 或 [`AuthHeader::None`] 返回 `None`。
    pub fn header_value(self, api_key: &str) -> Option<(String, String)> {
        if api_key.trim().is_empty() {
            return None;
        }
        match self {
            Self::None => None,
            Self::ApiKey => Some((self.header_name().to_string(), api_key.to_string())),
            Self::Bearer => Some((self.header_name().to_string(), format!("Bearer {api_key}"))),
        }
    }
}

/// `User-Agent`，格式照 `packages/ai/src/utils/pi-user-agent.ts:17-19`：`pi (<os> <release>; <arch>)`。
pub fn user_agent() -> String {
    format!(
        "pi ({} {}; {})",
        std::env::consts::OS,
        std::env::consts::ARCH,
        "x86_64-unknown-linux-musl"
    )
}

/// `/v1/chat/completions` 的 HTTP 客户端。
///
/// 只做传输与解码，**不做重试**——重试由 [`crate::retry::RetryPolicy`] 在上层驱动，
/// 与 TS 侧 `retryProviderRequest` 接受一个请求工厂的分工一致。
#[derive(Debug, Clone)]
pub struct ChatCompletionsClient {
    base_url: String,
    rates: Rates,
    /// provider 名，来自 `models.json`；会写进每条助手消息。
    provider: String,
    auth: AuthHeader,
    api_key: String,
    /// 模型自带头（对应 `api/openai-completions.ts:760` 的 `model.headers`）。
    headers: Vec<(String, String)>,
    cancel: CancellationToken,
}

impl ChatCompletionsClient {
    /// 以 `base_url`（形如 `http://host:port`）与费率构造。
    pub fn new(base_url: impl Into<String>, rates: Rates) -> Self {
        Self {
            base_url: base_url.into(),
            rates,
            provider: "mock".to_string(),
            auth: AuthHeader::default(),
            api_key: String::new(),
            headers: Vec::new(),
            cancel: CancellationToken::new(),
        }
    }

    /// 追加模型自带头（TS 里 `model.headers` 会被并入 `defaultHeaders`）。
    #[must_use]
    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    /// 给**解码器**装一个取消令牌（P4-15 步骤 4.2 的结论）。
    ///
    /// **生产路径不使用它**：abort 由 pi-core 的 `tokio::select!`（`crates/pi-core/src/agent/run.rs`
    /// 的"等下一个事件 vs cancel.cancelled()"）负责——abort 时它停止取事件并返回，
    /// 于是**流被丢弃**，reqwest 随之关闭连接、在途传输结束。
    /// 这个接口的用途是：① 嵌入式调用方自己持有令牌时可以直接取消解码；② 测试用它覆盖
    /// 解码器的取消分支（`crates/pi-ai/tests/t2_5_cancel.rs`、`t2_16_abort_mid_stream.rs` 等）。
    /// 行为与 ref 等价：ref 是把 `AbortSignal` 透传给 fetch（效果同为"请求不再继续"）。
    /// 指定认证头形态与 key。
    #[must_use]
    pub fn with_auth(mut self, auth: AuthHeader, api_key: &str) -> Self {
        self.auth = auth;
        self.api_key = api_key.to_string();
        self
    }

    /// 从 `models.json` 解析出的模型构造，消除硬编码的 provider 名。
    #[must_use]
    pub fn from_resolved(resolved: &crate::config::ResolvedModel) -> Self {
        Self {
            base_url: resolved.base_url.clone(),
            rates: resolved.rates,
            provider: resolved.provider_name.clone(),
            auth: AuthHeader::default(),
            api_key: String::new(),
            headers: Vec::new(),
            cancel: CancellationToken::new(),
        }
    }

    /// 当前使用的 provider 名。
    pub fn provider_name(&self) -> &str {
        &self.provider
    }

    /// 当前使用的 base url。
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 绑定取消令牌：取消会中断正在进行的请求。
    #[must_use]
    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = token;
        self
    }

    fn endpoint(&self) -> String {
        // `baseUrl` **原样使用**，只接上 `/chat/completions`——TS 的 SDK 就是这么拼的。
        // 我原先又加了一段 `/v1`，于是 `baseUrl` 已含版本段时会出现
        // `/v1/v1/chat/completions`（golden/S01 的请求路径是 `/v1/chat/completions`，
        // 由 `compare.mjs` 的 C 层比对抓出）。
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

/// 把解码器的最终消息写进共享槽位；锁中毒时不 panic（这只是诊断信息）。
fn store_final(
    slot: &std::sync::Arc<std::sync::Mutex<Option<crate::AssistantMessage>>>,
    decoder: &crate::decoder::StreamDecoder,
) {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(decoder.final_message());
    }
}

/// 从待发队列里取出一个事件；队列非空是调用前提。
fn event_of(pending: &mut VecDeque<AssistantEvent>) -> AssistantEvent {
    pending.pop_front().unwrap_or(AssistantEvent::Error {
        reason: crate::StopReason::Error,
        message: String::from("内部错误：事件队列为空"),
    })
}

impl Provider for ChatCompletionsClient {
    fn stream<'a>(
        &'a self,
        request: ProviderRequest,
    ) -> BoxFuture<'a, Result<Stream, ProviderError>> {
        Box::pin(async move {
            let body = build_chat_completions_request_with_provider(&request, &self.provider);
            // 不设 `.timeout()`——它会覆盖整个响应体，与 SDK 语义不符（见常量文档）。
            let http: reqwest::Client = reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .map_err(|error| {
                    ProviderError::transport(ProviderError::describe_transport_error(&error))
                })?;
            let mut builder = http.post(self.endpoint());
            if let Some((name, value)) = self.auth.header_value(&self.api_key) {
                builder = builder.header(name, value);
            }
            // **模型自带头 + 请求级头**（P4-15 步骤 4.1）。此前这里只发 `user-agent` + 鉴权头，
            // `self.headers`（来自 models.json 的 provider/model `headers`）**存了却从不使用**——
            // 这就是"extra 吞配置"在 openai 侧的落点。
            // 顺序照 ref：默认 `User-Agent`（`ai/src/api/openai-completions.ts:760`
            // `{"User-Agent": getPiUserAgent(), ...model.headers}`）→ 配置头 → 请求级头，
            // 同名后者覆盖前者（`model-runtime.ts:593` 的 `mergeHeaders`）。
            let mut headers: Vec<(String, String)> = vec![("user-agent".to_string(), user_agent())];
            for (name, value) in self.headers.iter().chain(request.headers.iter()) {
                headers.retain(|(existing, _not_value)| !existing.eq_ignore_ascii_case(name));
                headers.push((name.clone(), value.clone()));
            }
            for (name, value) in &headers {
                builder = builder.header(name, value);
            }
            // 只对"等到响应头"计时；`send()` 返回后计时即结束。
            let response =
                tokio::time::timeout(RESPONSE_HEADER_TIMEOUT, builder.json(&body).send())
                    .await
                    .map_err(|_| {
                        ProviderError::transport(format!(
                            "等待响应头超时（{}s）",
                            RESPONSE_HEADER_TIMEOUT.as_secs()
                        ))
                    })?
                    .map_err(|error| {
                        ProviderError::transport(ProviderError::describe_transport_error(&error))
                    })?;

            let status = response.status();
            if !status.is_success() {
                let retry_after_header = response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(crate::provider::RetryAfter::parse);
                let text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| String::from("<读取错误响应失败>"));
                let retry_after = retry_after_header;
                let reason = status.canonical_reason().unwrap_or("Error").to_string();
                return Err(ProviderError::http(
                    status.as_u16(),
                    reason,
                    text,
                    retry_after,
                ));
            }

            let decoder = StreamDecoder::new(
                OPENAI_COMPLETIONS,
                &self.provider,
                &request.model,
                self.rates,
            )
            .map_err(|error| ProviderError::stream_error(error.to_string()))?
            .with_cancel(self.cancel.clone());
            let bytes = response.bytes_stream();

            // 用 `unfold` 把「字节流 → 解码器 → 事件」串起来；不引 async-stream 之类的宏依赖。
            // 共享槽位由闭包捕获（**不**进 unfold 的状态元组——那会改动元组元数，
            // 让输出类型的推断失效，试过一次，回退了）。
            let final_slot: std::sync::Arc<std::sync::Mutex<Option<crate::AssistantMessage>>> =
                std::sync::Arc::new(std::sync::Mutex::new(None));
            let slot_in = final_slot.clone();
            let stream = futures::stream::unfold(
                (decoder, bytes, false, VecDeque::new()),
                move |(mut decoder, mut bytes, done, mut pending): (
                    _,
                    _,
                    _,
                    VecDeque<AssistantEvent>,
                )| {
                    let slot = slot_in.clone();
                    async move {
                        if done {
                            return None;
                        }
                        'stream: loop {
                            if let Some(event) = pending.pop_front() {
                                return Some((event, (decoder, bytes, false, pending)));
                            }

                            match bytes.next().await {
                                Some(Ok(chunk)) => match decoder.push(&chunk) {
                                    Ok(events) => {
                                        // **这条路才是真实的逐块解码路径**：每次解出一批事件后
                                        // 刷新快照，流式期间 `final_message()` 才是当前累积消息。
                                        // 探针实测：不写这里时整个流式过程都是 `Some=false`。
                                        store_final(&slot, &decoder);
                                        pending.extend(events);
                                    }
                                    Err(crate::decoder::DecodeError::Aborted) => {
                                        // 取消不是错误事件：TS 只把助手消息标成 aborted，
                                        // 事件流里不会出现 error（对照 golden/S09）。
                                        // 但**最终消息要带出去**（aborted + 已收到的部分内容）——
                                        // 这里用 break 而不是 `return None`：早退会让 unfold 的输出类型
                                        // 推断失效（上一轮试过，见 notes/decisions.md）。
                                        store_final(&slot, &decoder);
                                        break 'stream;
                                    }
                                    Err(error) => {
                                        // 流内失败按 TS 的形态落成一个终止的 error 事件，
                                        // 上层据此把这条助手消息标成 error。
                                        pending.push_back(AssistantEvent::Error {
                                            reason: crate::StopReason::Error,
                                            message: error.to_string(),
                                        });
                                        // **每个事件之后都刷新快照**：流式期间 `final_message()`
                                        // 返回的是**当前累积消息**，json 模式靠它逐增量发
                                        // `message_update`（`ai/src/types.ts:653-668` 的 `partial`）。
                                        // 原先只在终止路径写槽位，于是线上只有 start/end、没有增量。
                                        // 注意必须写在 `return` **之前**——我第一版写在后面，
                                        // 成了不可达代码，clippy 直接报 `unreachable statement`。
                                        store_final(&slot, &decoder);
                                        return Some((
                                            event_of(&mut pending),
                                            (decoder, bytes, true, pending),
                                        ));
                                    }
                                },
                                Some(Err(error)) => {
                                    pending.push_back(AssistantEvent::Error {
                                        reason: crate::StopReason::Error,
                                        message: error.to_string(),
                                    });
                                    return Some((
                                        event_of(&mut pending),
                                        (decoder, bytes, true, pending),
                                    ));
                                }
                                None => {
                                    return match decoder.finish() {
                                        Ok(events) if events.is_empty() => {
                                            store_final(&slot, &decoder);
                                            None
                                        }
                                        Ok(mut events) => {
                                            // **这条路才是 mock（整段响应一次到达）实际走的**：
                                            // 事件一次性从 `finish()` 出来。原先这里不写槽位，
                                            // 于是整个流式期间 `final_message()` 都是 `None`，
                                            // 循环发不出 `message_update`。
                                            // 探针实测：Start/TextDelta/Done 期间全部 `Some=false`。
                                            store_final(&slot, &decoder);
                                            let first = events.remove(0);
                                            pending.extend(events);
                                            Some((first, (decoder, bytes, false, pending)))
                                        }
                                        Err(error) => {
                                            pending.push_back(AssistantEvent::Error {
                                                reason: crate::StopReason::Error,
                                                message: error.to_string(),
                                            });
                                            Some((
                                                event_of(&mut pending),
                                                (decoder, bytes, true, pending),
                                            ))
                                        }
                                    };
                                }
                            }
                        }
                        None
                    }
                },
            );

            let stream: futures::stream::BoxStream<'static, AssistantEvent> = Box::pin(stream);
            Ok(Stream::with_final_message(stream, final_slot))
        })
    }
}
