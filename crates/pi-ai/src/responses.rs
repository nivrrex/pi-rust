//! `openai-responses` api（ref `packages/ai/src/api/openai-responses.ts` +
//! `openai-responses-shared.ts`）。
//!
//! 与 `openai-completions` 的差别：
//! - 端点 `POST {baseUrl}/responses`（`:160` 由 SDK 拼）；
//! - 请求体用 `input` 条目数组（`role:system|user|assistant` + `function_call` /
//!   `function_call_output`），系统提示**写成字符串**；`store:false`、`stream:true`、
//!   `max_output_tokens`、`tools:[{type:"function",name,description,parameters}]`；
//! - 流是**语义化事件**（`response.output_text.delta` 等），不是 delta chunks。

use futures::{stream::BoxStream, StreamExt};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;

use crate::openai::{user_agent, AuthHeader, CONNECT_TIMEOUT, RESPONSE_HEADER_TIMEOUT};
use crate::provider::{Provider, ProviderError, ProviderRequest, Stream};
use crate::types::Cost;
use crate::types::{AssistantEvent, AssistantMessage, Content, StopReason, Usage};

/// 端点路径（ref 由 OpenAI SDK 的 `responses.create` 拼出 `/responses`）。
fn endpoint(base_url: &str) -> String {
    format!("{}/responses", base_url.trim_end_matches('/'))
}

/// 把上下文消息转成 responses 的 `input` 条目（ref `openai-responses-shared.ts:240-420`）。
#[must_use]
pub fn build_input(context: &crate::context::Context) -> Vec<Value> {
    let mut items: Vec<Value> = Vec::new();
    if let Some(system) = context.system.as_deref().filter(|text| !text.is_empty()) {
        // ref：系统提示写成**字符串** content（不是 input_text 数组）。
        items.push(json!({"role": "system", "content": system}));
    }
    for message in &context.messages {
        match message {
            crate::message::Message::User { content, .. } => {
                let text = match content {
                    crate::UserContent::Text(text) => text.clone(),
                    crate::UserContent::Blocks(blocks) => blocks
                        .iter()
                        .filter_map(|block| match block {
                            Content::Text { text, .. } => Some(text.as_str()),
                            _other => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                items.push(json!({
                    "role": "user",
                    "content": [{"type": "input_text", "text": text}],
                }));
            }
            crate::message::Message::Assistant(assistant) => {
                let text: String = assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        Content::Text { text, .. } => Some(text.as_str()),
                        _other => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    items.push(json!({
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": text}],
                    }));
                }
                for block in &assistant.content {
                    if let Content::ToolCall {
                        id,
                        name,
                        arguments,
                        ..
                    } = block
                    {
                        items.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": Value::Object(arguments.clone()).to_string(),
                        }));
                    }
                }
            }
            crate::message::Message::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                let text: String = content
                    .iter()
                    .filter_map(|block| match block {
                        crate::message::ToolResultContent::Text { text, .. } => Some(text.as_str()),
                        _other => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": text,
                }));
            }
            _other => {}
        }
    }
    items
}

/// 构造 `POST /responses` 的请求体（字段集合取自 TS 实测：
/// `model/input/stream/store/max_output_tokens/tools/prompt_cache_key`）。
#[must_use]
pub fn build_responses_request(request: &ProviderRequest, _provider: &str) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(request.model));
    body.insert(
        "input".to_string(),
        Value::Array(build_input(&request.context)),
    );
    body.insert("stream".to_string(), json!(true));
    body.insert("store".to_string(), json!(false));
    body.insert(
        "max_output_tokens".to_string(),
        json!(request.max_completion_tokens),
    );
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_string(),
            Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        })
                    })
                    .collect(),
            ),
        );
    }
    // 思考等级：responses 用 `reasoning.effort`（ref `openai-responses-shared.ts` 的
    // `reasoning: {effort}`；`minimal/low/medium/high` 原样下发）。
    if let Some(effort) = &request.context.reasoning_effort {
        body.insert("reasoning".to_string(), json!({"effort": effort}));
    }
    Value::Object(body)
}

/// `openai-responses` 客户端。
pub struct ResponsesClient {
    base_url: String,
    rates: crate::decoder::Rates,
    provider: String,
    auth: AuthHeader,
    api_key: String,
    headers: Vec<(String, String)>,
    cancel: tokio_util::sync::CancellationToken,
}

impl ResponsesClient {
    /// 以 `base_url` 与费率构造。
    #[must_use]
    pub fn new(base_url: impl Into<String>, rates: crate::decoder::Rates) -> Self {
        Self {
            base_url: base_url.into(),
            rates,
            provider: "mock".to_string(),
            auth: AuthHeader::default(),
            api_key: String::new(),
            headers: Vec::new(),
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// 从解析出的模型构造。
    #[must_use]
    pub fn from_resolved(resolved: &crate::config::ResolvedModel) -> Self {
        Self {
            base_url: resolved.base_url.clone(),
            rates: resolved.rates,
            provider: resolved.provider_name.clone(),
            auth: AuthHeader::default(),
            api_key: String::new(),
            headers: Vec::new(),
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// 指定认证头形态与 key。
    #[must_use]
    pub fn with_auth(mut self, auth: AuthHeader, api_key: &str) -> Self {
        self.auth = auth;
        self.api_key = api_key.to_string();
        self
    }

    /// 追加模型自带头。
    #[must_use]
    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    /// 绑定取消令牌。
    #[must_use]
    pub fn with_cancel(mut self, token: tokio_util::sync::CancellationToken) -> Self {
        self.cancel = token;
        self
    }
}

impl Provider for ResponsesClient {
    fn stream<'a>(
        &'a self,
        request: ProviderRequest,
    ) -> futures::future::BoxFuture<'a, Result<Stream, ProviderError>> {
        Box::pin(async move {
            let body = build_responses_request(&request, &self.provider);
            let http = reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .map_err(|error| {
                    ProviderError::transport(ProviderError::describe_transport_error(&error))
                })?;
            let mut builder = http.post(endpoint(&self.base_url));
            if let Some((name, value)) = self.auth.header_value(&self.api_key) {
                builder = builder.header(name, value);
            }
            let mut headers: Vec<(String, String)> = vec![("user-agent".to_string(), user_agent())];
            for (name, value) in self.headers.iter().chain(request.headers.iter()) {
                headers.retain(|(existing, _not_value)| !existing.eq_ignore_ascii_case(name));
                headers.push((name.clone(), value.clone()));
            }
            for (name, value) in &headers {
                builder = builder.header(name, value);
            }
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
                let reason = status.canonical_reason().unwrap_or("Error").to_string();
                return Err(ProviderError::http(
                    status.as_u16(),
                    reason,
                    text,
                    retry_after_header,
                ));
            }

            // 第二个参数是 **API 类型**（`AssistantMessage.api`），不是 provider 名。
            // ref 的 assistant 消息里 `api: "openai-responses"`；曾经这里传了 provider 名，
            // 打印/事件里就变成 `api: "mock"`（X3 对拍抓到的差异）。
            let mut decoder = ResponsesDecoder::new(
                self.provider.clone(),
                "openai-responses".to_string(),
                request.model.clone(),
                self.rates,
            );
            let bytes = response.bytes_stream();
            let final_slot: std::sync::Arc<std::sync::Mutex<Option<AssistantMessage>>> =
                std::sync::Arc::new(std::sync::Mutex::new(None));
            let slot_in = final_slot.clone();
            let stream = futures::stream::unfold(
                (decoder_take(&mut decoder), bytes, false, VecDeque::new()),
                move |(mut decoder, mut bytes, done, mut pending): (
                    ResponsesDecoder,
                    _,
                    bool,
                    VecDeque<AssistantEvent>,
                )| {
                    let slot = slot_in.clone();
                    async move {
                        if done {
                            return None;
                        }
                        loop {
                            if let Some(event) = pending.pop_front() {
                                return Some((event, (decoder, bytes, false, pending)));
                            }
                            match bytes.next().await {
                                Some(Ok(chunk)) => match decoder.push(&chunk) {
                                    Ok(events) => {
                                        store(&slot, &decoder);
                                        pending.extend(events);
                                    }
                                    Err(error) => {
                                        pending.push_back(AssistantEvent::Error {
                                            reason: StopReason::Error,
                                            message: error,
                                        });
                                        store(&slot, &decoder);
                                        let first =
                                            pending.pop_front().unwrap_or(AssistantEvent::Error {
                                                reason: StopReason::Error,
                                                message: String::from("内部错误"),
                                            });
                                        return Some((first, (decoder, bytes, true, pending)));
                                    }
                                },
                                Some(Err(error)) => {
                                    pending.push_back(AssistantEvent::Error {
                                        reason: StopReason::Error,
                                        message: error.to_string(),
                                    });
                                    store(&slot, &decoder);
                                    let first =
                                        pending.pop_front().unwrap_or(AssistantEvent::Error {
                                            reason: StopReason::Error,
                                            message: String::from("内部错误"),
                                        });
                                    return Some((first, (decoder, bytes, true, pending)));
                                }
                                None => {
                                    let events = decoder.finish();
                                    store(&slot, &decoder);
                                    if events.is_empty() {
                                        return None;
                                    }
                                    let mut events = events;
                                    let first = events.remove(0);
                                    pending.extend(events);
                                    return Some((first, (decoder, bytes, false, pending)));
                                }
                            }
                        }
                    }
                },
            );
            let stream: BoxStream<'static, AssistantEvent> = Box::pin(stream);
            Ok(Stream::with_final_message(stream, final_slot))
        })
    }
}

fn decoder_take(decoder: &mut ResponsesDecoder) -> ResponsesDecoder {
    std::mem::replace(decoder, ResponsesDecoder::placeholder())
}

fn store(
    slot: &std::sync::Arc<std::sync::Mutex<Option<AssistantMessage>>>,
    decoder: &ResponsesDecoder,
) {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(decoder.message());
    }
}

/// `openai-responses` 的流式解码器（语义事件 → [`AssistantEvent`]）。
pub struct ResponsesDecoder {
    provider: String,
    api: String,
    model: String,
    rates: crate::decoder::Rates,
    buffer: Vec<u8>,
    message: AssistantMessage,
    /// 每个 `output_index` 对应的内容块下标与类型。
    slots: Vec<Option<(usize, &'static str)>>,
    /// 工具调用的 partial JSON 累积。
    tool_partial: Vec<String>,
    started: bool,
    saw_terminal: bool,
    finished: bool,
}

impl ResponsesDecoder {
    /// 构造。
    #[must_use]
    pub fn new(provider: String, api: String, model: String, rates: crate::decoder::Rates) -> Self {
        Self {
            provider,
            api,
            model,
            rates,
            buffer: Vec::new(),
            message: AssistantMessage {
                role: "assistant".to_string(),
                content: Vec::new(),
                api: String::new(),
                provider: String::new(),
                model: String::new(),
                response_model: None,
                usage: Usage::default(),
                stop_reason: StopReason::Pending,
                error_message: None,
                raw_stop_reason: None,
                end_turn: None,
                response_id: None,
                timestamp: 0,
            },
            slots: Vec::new(),
            tool_partial: Vec::new(),
            started: false,
            saw_terminal: false,
            finished: false,
        }
    }

    fn placeholder() -> Self {
        Self::new(
            String::new(),
            String::new(),
            String::new(),
            crate::decoder::Rates::zero(),
        )
    }

    /// 当前累积的助手消息（流式期间可反复取）。
    #[must_use]
    pub fn message(&self) -> AssistantMessage {
        self.message.clone()
    }

    fn slot(&mut self, index: usize, kind: &'static str) -> Option<usize> {
        while self.slots.len() <= index {
            self.slots.push(None);
        }
        match self.slots[index] {
            Some((content_index, existing)) if existing == kind => Some(content_index),
            _other => None,
        }
    }

    fn create_slot(&mut self, index: usize, kind: &'static str) -> usize {
        while self.slots.len() <= index {
            self.slots.push(None);
        }
        if let Some((content_index, _old)) = self.slots[index] {
            return content_index;
        }
        let content_index = self.message.content.len();
        self.message.content.push(match kind {
            "thinking" => Content::Thinking {
                thinking: String::new(),
                thinking_signature: None,
                redacted: None,
            },
            "toolCall" => Content::ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: Map::new(),
                thought_signature: None,
            },
            _other => Content::Text {
                text: String::new(),
                text_signature: None,
            },
        });
        self.slots[index] = Some((content_index, kind));
        content_index
    }

    fn push_event(&mut self, value: &Value, out: &mut Vec<AssistantEvent>) {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "response.created" => {
                if let Some(id) = value
                    .get("response")
                    .and_then(|response| response.get("id"))
                    .and_then(Value::as_str)
                {
                    self.message.response_id = Some(id.to_string());
                }
            }
            "response.output_item.added" => {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let item = value.get("item").cloned().unwrap_or(Value::Null);
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                match item_type {
                    "reasoning" => {
                        let content_index = self.create_slot(index, "thinking");
                        out.push(AssistantEvent::ThinkingStart { content_index });
                    }
                    "function_call" | "custom_tool_call" => {
                        let content_index = self.create_slot(index, "toolCall");
                        if let Some(Content::ToolCall { id, name, .. }) =
                            self.message.content.get_mut(content_index)
                        {
                            *id = item
                                .get("call_id")
                                .or_else(|| item.get("id"))
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            *name = item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                        }
                        while self.tool_partial.len() <= content_index {
                            self.tool_partial.push(String::new());
                        }
                        out.push(AssistantEvent::ToolCallStart { content_index });
                    }
                    _other => {
                        let content_index = self.create_slot(index, "text");
                        out.push(AssistantEvent::TextStart { content_index });
                    }
                }
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let delta = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if let Some(content_index) = self.slot(index, "text") {
                    if let Some(Content::Text { text, .. }) =
                        self.message.content.get_mut(content_index)
                    {
                        text.push_str(&delta);
                    }
                    out.push(AssistantEvent::TextDelta {
                        content_index,
                        delta,
                    });
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let delta = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if let Some(content_index) = self.slot(index, "thinking") {
                    if let Some(Content::Thinking { thinking, .. }) =
                        self.message.content.get_mut(content_index)
                    {
                        thinking.push_str(&delta);
                    }
                    out.push(AssistantEvent::ThinkingDelta {
                        content_index,
                        delta,
                    });
                }
            }
            "response.reasoning_summary_part.done" => {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(content_index) = self.slot(index, "thinking") {
                    if let Some(Content::Thinking { thinking, .. }) =
                        self.message.content.get_mut(content_index)
                    {
                        thinking.push_str("\n\n");
                    }
                    out.push(AssistantEvent::ThinkingDelta {
                        content_index,
                        delta: String::from("\n\n"),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let delta = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if let Some(content_index) = self.slot(index, "toolCall") {
                    while self.tool_partial.len() <= content_index {
                        self.tool_partial.push(String::new());
                    }
                    self.tool_partial[content_index].push_str(&delta);
                    if let Some(Content::ToolCall { arguments, .. }) =
                        self.message.content.get_mut(content_index)
                    {
                        if let Value::Object(map) =
                            parse_streaming_json(&self.tool_partial[content_index])
                        {
                            *arguments = map;
                        }
                    }
                    out.push(AssistantEvent::ToolCallDelta {
                        content_index,
                        delta,
                    });
                }
            }
            "response.function_call_arguments.done" => {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(content_index) = self.slot(index, "toolCall") {
                    let arguments = value
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if let Some(Content::ToolCall {
                        arguments: parsed, ..
                    }) = self.message.content.get_mut(content_index)
                    {
                        if let Value::Object(map) = parse_streaming_json(&arguments) {
                            *parsed = map;
                        }
                    }
                    out.push(AssistantEvent::ToolCallDelta {
                        content_index,
                        delta: String::new(),
                    });
                }
            }
            "response.output_item.done" => {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let item = value.get("item").cloned().unwrap_or(Value::Null);
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                match item_type {
                    "reasoning" => {
                        if let Some(content_index) = self.slot(index, "thinking") {
                            let summary = item
                                .get("summary")
                                .and_then(Value::as_array)
                                .map(|parts| {
                                    parts
                                        .iter()
                                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                                        .collect::<Vec<_>>()
                                        .join("\n\n")
                                })
                                .unwrap_or_default();
                            let content = item
                                .get("content")
                                .and_then(Value::as_array)
                                .map(|parts| {
                                    parts
                                        .iter()
                                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                                        .collect::<Vec<_>>()
                                        .join("\n\n")
                                })
                                .unwrap_or_default();
                            let final_text = if summary.is_empty() { content } else { summary };
                            if let Some(Content::Thinking {
                                thinking,
                                thinking_signature,
                                ..
                            }) = self.message.content.get_mut(content_index)
                            {
                                if !final_text.is_empty() {
                                    *thinking = final_text.clone();
                                }
                                *thinking_signature = Some(item.to_string());
                            }
                            out.push(AssistantEvent::ThinkingEnd {
                                content_index,
                                content: final_text,
                            });
                        }
                    }
                    "function_call" | "custom_tool_call" => {
                        if let Some(content_index) = self.slot(index, "toolCall") {
                            if let Some(Content::ToolCall { arguments, .. }) =
                                self.message.content.get_mut(content_index)
                            {
                                if let Some(text) = item.get("arguments").and_then(Value::as_str) {
                                    if let Value::Object(map) = parse_streaming_json(text) {
                                        *arguments = map;
                                    }
                                }
                            }
                            out.push(AssistantEvent::ToolCallEnd { content_index });
                        }
                    }
                    _other => {
                        if let Some(content_index) = self.slot(index, "text") {
                            let content = item
                                .get("content")
                                .and_then(Value::as_array)
                                .map(|parts| {
                                    parts
                                        .iter()
                                        .filter_map(|part| {
                                            part.get("text")
                                                .or_else(|| part.get("refusal"))
                                                .and_then(Value::as_str)
                                        })
                                        .collect::<Vec<_>>()
                                        .join("")
                                })
                                .unwrap_or_default();
                            if !content.is_empty() {
                                if let Some(Content::Text { text, .. }) =
                                    self.message.content.get_mut(content_index)
                                {
                                    *text = content.clone();
                                }
                            }
                            out.push(AssistantEvent::TextEnd {
                                content_index,
                                content,
                            });
                        }
                    }
                }
            }
            "response.completed" | "response.incomplete" => {
                self.saw_terminal = true;
                self.finalize(value.get("response").unwrap_or(&Value::Null));
                out.push(AssistantEvent::Done {
                    reason: self.message.stop_reason,
                });
            }
            "response.failed" => {
                self.saw_terminal = true;
                let response = value.get("response").cloned().unwrap_or(Value::Null);
                let message = response
                    .get("error")
                    .map(|error| {
                        let code = error
                            .get("code")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown");
                        let text = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("no message");
                        format!("{code}: {text}")
                    })
                    .or_else(|| {
                        response
                            .get("incomplete_details")
                            .and_then(|details| details.get("reason"))
                            .and_then(Value::as_str)
                            .map(|reason| format!("incomplete: {reason}"))
                    })
                    .unwrap_or_else(|| {
                        String::from("Unknown error (no error details in response)")
                    });
                self.message.stop_reason = StopReason::Error;
                self.message.error_message = Some(message.clone());
                out.push(AssistantEvent::Error {
                    reason: StopReason::Error,
                    message,
                });
            }
            "error" => {
                self.saw_terminal = true;
                let code = value
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let text = value
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                let message = format!("{code}: {text}");
                self.message.stop_reason = StopReason::Error;
                self.message.error_message = Some(message.clone());
                out.push(AssistantEvent::Error {
                    reason: StopReason::Error,
                    message,
                });
            }
            _other => {}
        }
    }

    fn finalize(&mut self, response: &Value) {
        if let Some(id) = response.get("id").and_then(Value::as_str) {
            self.message.response_id = Some(id.to_string());
        }
        if let Some(usage) = response.get("usage") {
            let cached = usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let cache_write = usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cache_write_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let input = usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_sub(cached)
                .saturating_sub(cache_write);
            self.message.usage = Usage {
                input,
                output: usage
                    .get("output_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                cache_read: cached,
                cache_write,
                cache_write_1h: None,
                reasoning: usage
                    .get("output_tokens_details")
                    .and_then(|details| details.get("reasoning_tokens"))
                    .and_then(Value::as_u64),
                total_tokens: usage
                    .get("total_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                cost: Cost::default(),
            };
            self.message.usage.cost =
                crate::decoder::calculate_cost(self.rates, &self.message.usage);
        }
        let status = response
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed");
        let incomplete = response
            .get("incomplete_details")
            .and_then(|details| details.get("reason"))
            .and_then(Value::as_str);
        self.message.raw_stop_reason = Some(match incomplete {
            Some(reason) => format!("{status}.{reason}"),
            None => status.to_string(),
        });
        self.message.stop_reason = match status {
            "completed" => StopReason::Stop,
            "incomplete" => {
                if incomplete == Some("max_output_tokens") {
                    StopReason::Length
                } else {
                    self.message.error_message = Some(match incomplete {
                        Some(reason) => format!("Response incomplete: {reason}"),
                        None => String::from("Response incomplete without a provider reason"),
                    });
                    StopReason::Error
                }
            }
            "in_progress" | "queued" => StopReason::Stop,
            "failed" | "cancelled" => StopReason::Error,
            _other => StopReason::Stop,
        };
        if self
            .message
            .content
            .iter()
            .any(|block| matches!(block, Content::ToolCall { .. }))
            && matches!(self.message.stop_reason, StopReason::Stop)
        {
            self.message.stop_reason = StopReason::ToolUse;
        }
    }

    /// 喂一段字节，返回解出的事件。
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<AssistantEvent>, String> {
        self.buffer.extend_from_slice(chunk);
        let mut out = Vec::new();
        if !self.started {
            self.started = true;
            self.message.api = self.api.clone();
            self.message.provider = self.provider.clone();
            self.message.model = self.model.clone();
            self.message.timestamp = now_millis();
            out.push(AssistantEvent::Start);
        }
        // SSE：以空行分隔事件；每个事件里取 `data: ` 行。
        while let Some(position) = find_double_newline(&self.buffer) {
            let event: Vec<u8> = self.buffer.drain(..position + 2).collect();
            let text = String::from_utf8_lossy(&event);
            for line in text.lines() {
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(data) {
                    self.push_event(&value, &mut out);
                }
            }
        }
        Ok(out)
    }

    /// 流结束：补齐终止事件（ref 的 "stream ended before a terminal response event"）。
    pub fn finish(&mut self) -> Vec<AssistantEvent> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        if !self.started {
            return Vec::new();
        }
        if self.saw_terminal {
            return Vec::new();
        }
        let message =
            String::from("OpenAI Responses stream ended before a terminal response event");
        self.message.stop_reason = StopReason::Error;
        self.message.error_message = Some(message.clone());
        vec![AssistantEvent::Error {
            reason: StopReason::Error,
            message,
        }]
    }
}

fn find_double_newline(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|window| window == b"\n\n")
}

/// 当前毫秒时间戳（与 `decoder.rs` 的口径一致）。
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// 解析"流式 JSON"（半截 JSON 也尽量给出对象；ref `parseStreamingJson`）。
fn parse_streaming_json(text: &str) -> Value {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return value;
    }
    // 逐步截断到最后一个能解析的前缀（ref 同款启发式：补括号 + 去尾逗号）。
    let trimmed = text.trim_end().trim_end_matches(',');
    for suffix in ["", "}", "]}", "\"}", "\"}}"] {
        let candidate = format!("{trimmed}{suffix}");
        if let Ok(value) = serde_json::from_str::<Value>(&candidate) {
            return value;
        }
    }
    Value::Object(Map::new())
}
