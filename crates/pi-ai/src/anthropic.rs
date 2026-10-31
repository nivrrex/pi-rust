//! `anthropic-messages` 适配层，对照 `golden/S12-anthropic`、`golden/S25-anthropic`
//! 的请求体与 `ref/pi-0.87.1/packages/ai/src/api/anthropic-messages.ts`（见 `spec/a-types.md`）。

use serde_json::{json, Map, Value};

use crate::ProviderRequest;

/// Anthropic 的工具声明。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpecAnthropic {
    pub name: String,
    pub description: String,
    /// 原样透传的 JSON Schema（Anthropic 叫 `input_schema`）。
    pub input_schema: Value,
    /// 对应 compat 的 `supportsEagerToolInputStreaming`；关着也要显式发 `false`。
    pub eager_input_streaming: bool,
}

/// 构造 `/v1/messages` 的请求体。
///
/// 与 openai-completions 的差别（都来自 golden 的实际请求体）：
/// - `system` 是**块数组**，不是字符串；
/// - 有 `max_tokens` 而非 `max_completion_tokens`；
/// - user 文本块带 `cache_control: {type: "ephemeral"}`；
/// - 工具用 `name` / `description` / `input_schema`，且带 `eager_input_streaming`。
pub fn build_messages_request(request: &ProviderRequest, tools: &[ToolSpecAnthropic]) -> Value {
    build_messages_request_with_thinking(request, tools, None)
}

/// Anthropic 的 thinking 配置。
///
/// 规则照 `api/anthropic-messages.ts:1160-1182`：
/// - `enabled` 形态：`{type:"enabled", budget_tokens, display}`，**缺省 budget 是 1024**
///   （`options.thinkingBudgetTokens || 1024`）、display 缺省 `summarized`；
/// - `disabled` 形态：`{type:"disabled"}`；
/// - `adaptive` 形态：`{type:"adaptive", display}`，可选带 `output_config.effort`
///   （仅当 compat 的 `forceAdaptiveThinking` 为真）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingConfig {
    /// 预算制（老模型）。
    Enabled {
        /// 缺省 1024，与 TS 一致。
        budget_tokens: Option<u64>,
        /// 缺省 `summarized`。
        display: Option<&'static str>,
    },
    /// 自适应（`forceAdaptiveThinking`）。
    Adaptive {
        display: Option<&'static str>,
        /// `output_config.effort`；给了才发该字段。
        effort: Option<&'static str>,
    },
    /// 显式关闭。
    Disabled,
}

/// 拼出 anthropic 的 messages 端点（P4-14 步骤 3）。
///
/// **约定：`baseUrl` 原样交给 SDK，由 SDK 接 `/v1/messages?beta=true`。**
/// ref：
/// - `packages/ai/src/api/anthropic-messages.ts:923/945/982` 只做 `baseURL: model.baseUrl`；
/// - `@anthropic-ai/sdk/src/client.ts:986`
///   `new URL(baseURL + (baseURL.endsWith('/') && path.startsWith('/') ? path.slice(1) : path))`
///   —— 字符串拼接，只在"baseURL 以 `/` 结尾且 path 以 `/` 开头"时去掉 path 的首个 `/`；
/// - `@anthropic-ai/sdk/src/client.ts:632` 默认 `baseURL` 为 `https://api.anthropic.com`（**不含 `/v1`**）；
/// - `@anthropic-ai/sdk/src/resources/beta/messages/messages.ts:122`
///   `this._client.post('/v1/messages?beta=true', {…})`。
///
/// 我方此前拼的是 `{baseUrl}/messages?beta=true`（假定配置里已含 `/v1`）——对 origin 形态的
/// baseUrl 会漏掉 `/v1`。现在改成 `{baseUrl}/v1/messages?beta=true`，尾斜杠写法与不带斜杠**同值**。
pub fn messages_endpoint(base_url: &str) -> String {
    format!("{}/v1/messages?beta=true", base_url.trim_end_matches('/'))
}

/// 答案至少保留的 token 数（ref `api/simple-options.ts:55` `MIN_ANSWER_TOKENS`）。
pub const MIN_ANSWER_TOKENS: u64 = 1024;

/// 各思考等级的默认预算（ref `api/simple-options.ts:57-62` `DEFAULT_THINKING_BUDGETS`）。
///
/// **未实现**：`options.thinkingBudgets`（settings 的自定义预算，ref `types.ts:101-106`）——
/// pi-rs 尚无该 settings 项，故只用默认表。
const DEFAULT_THINKING_BUDGETS: [(&str, u64); 4] = [
    ("minimal", 1024),
    ("low", 2048),
    ("medium", 8192),
    ("high", 16384),
];

/// 等级 → 预算（ref `api/simple-options.ts:64-72`）。
///
/// `xhigh` / `max` 先经 `clampReasoning`（`:64-66`）折成 `high`。
/// 不在 `ThinkingLevel`（`types.ts:84`）里的字符串返回 `None`——**不猜**一个预算
/// （`set_thinking_level` 目前不校验等级，见 `notes/p413-hardcoded-scan.md`）。
pub fn thinking_budget_for_level(level: &str) -> Option<u64> {
    let clamped = match level {
        "xhigh" | "max" => "high",
        other => other,
    };
    DEFAULT_THINKING_BUDGETS
        .iter()
        .find(|(name, _not_budget)| *name == clamped)
        .map(|(_name, budget)| *budget)
}

/// 把预算夹到"答案至少留 [`MIN_ANSWER_TOKENS`]"（ref `api/simple-options.ts:75-77`）。
pub fn clamp_thinking_budget_to_answer_room(thinking_budget: u64, ceiling: u64) -> u64 {
    thinking_budget.min(ceiling.saturating_sub(MIN_ANSWER_TOKENS))
}

/// ref `api/simple-options.ts:79-95` `adjustMaxTokensForThinking`。
///
/// 返回 `(max_tokens, thinking_budget)`；`base_max_tokens` 为 `None` 表示调用方没有给上限
/// （用模型上限，并把思考塞进去）。
pub fn adjust_max_tokens_for_thinking(
    base_max_tokens: Option<u64>,
    model_max_tokens: u64,
    level: &str,
) -> Option<(u64, u64)> {
    let mut thinking_budget = thinking_budget_for_level(level)?;
    let max_tokens = match base_max_tokens {
        None => model_max_tokens,
        Some(base) => base.saturating_add(thinking_budget).min(model_max_tokens),
    };
    if max_tokens <= thinking_budget {
        thinking_budget = clamp_thinking_budget_to_answer_room(thinking_budget, max_tokens);
    }
    Some((max_tokens, thinking_budget))
}

/// `anthropic-beta` 里"交错思考"特性的字面值（ref `api/anthropic-messages.ts:182`）。
pub const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";
/// 细粒度工具流式（ref `:181`）。
pub const FINE_GRAINED_TOOL_STREAMING_BETA: &str = "fine-grained-tool-streaming-2025-05-14";

/// 从两组头里找 `anthropic-beta`（大小写不敏感；请求级覆盖客户端级，ref `:999-1003`）。
fn configured_beta_header<'a>(
    headers: impl Iterator<Item = &'a (String, String)>,
) -> Option<String> {
    headers
        .filter(|(name, _value)| name.eq_ignore_ascii_case("anthropic-beta"))
        .map(|(_name, value)| value.clone())
        .last()
}

/// 用户自己配的 `anthropic-beta` 按 `,` 拆分、去空白、去重（ref `:1005-1014`）。
fn split_beta_features(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for feature in value.split(',') {
        let feature = feature.trim();
        if !feature.is_empty() && !out.iter().any(|existing| existing == feature) {
            out.push(feature.to_string());
        }
    }
    out
}

/// 请求要带的 beta 特性（ref `:991-1033` `getBetaFeatures`；`:1074` 非空才发）。
///
/// pi-rs 能评估的触发条件：
/// - 用户配了 `anthropic-beta` 头 → **原样采用**（`:1005-1014`）；
/// - `model.reasoning && thinkingEnabled === true && (interleavedThinking ?? true) &&
///   compat.forceAdaptiveThinking !== true`（`:1019-1026`）→ 交错思考。
///   pi-rs 只在"模型是推理模型且等级非 off"时才给 `request.thinking` 填值
///   （`crates/pi-core/src/agent/run.rs:311`），所以这里用 `ThinkingConfig::Enabled` 当该谓词。
///
/// **未实现（无对应数据）**：`isOAuthToken`（pi-rs 无 OAuth）、
/// `shouldUseFineGrainedToolStreamingBeta`（需要 `!supportsEagerToolInputStreaming`，
/// pi-rs 只走缺省 true，故恒 false）、`shouldUseServerSideFallbackBeta` /
/// `supportsMidConvoEffort`（`models.json` 的 `compat` 未建模）、`nativeToolChanges`（未建模）。
/// 因此除交错思考外的特性目前都不会被加进来。
pub fn beta_features(request: &ProviderRequest, model_headers: &[(String, String)]) -> Vec<String> {
    let configured = configured_beta_header(model_headers.iter().chain(request.headers.iter()));
    if let Some(value) = configured {
        return split_beta_features(&value);
    }
    let mut features: Vec<String> = Vec::new();
    if matches!(request.thinking, Some(ThinkingConfig::Enabled { .. })) {
        features.push(INTERLEAVED_THINKING_BETA.to_string());
    }
    features
}

/// 组装 HTTP 请求头：客户端头 → 请求级头覆盖（ref `:433-440` 的"请求级优先"），
/// 非空 beta 特性补成 `anthropic-beta`（ref `:1074` 的 `betas` 经 SDK 变成该头）。
pub fn build_request_headers(
    client_headers: &[(String, String)],
    request_headers: &[(String, String)],
    beta_features: &[String],
) -> Vec<(String, String)> {
    let mut headers = client_headers.to_vec();
    for (name, value) in request_headers {
        headers.retain(|(existing, _not_value)| existing != name);
        headers.push((name.clone(), value.clone()));
    }
    if !beta_features.is_empty() {
        headers.retain(|(name, _not_value)| !name.eq_ignore_ascii_case("anthropic-beta"));
        headers.push(("anthropic-beta".to_string(), beta_features.join(",")));
    }
    headers
}

/// 思考等级 → 请求体里的 `thinking` 配置（ref `api/anthropic-messages.ts:858-904` + `:1160-1182`）。
///
/// - `level == None`：不发该字段（ref `:1160` 的 `else if (model.reasoning)` 之外）。
/// - `Some("off")`：`{ type: "disabled" }`（ref `:1179-1181`）。
/// - 其余等级：`{ type: "enabled", budget_tokens, display: "summarized" }`（ref `:1173-1177`），
///   其中预算再按 `:902` 取 `min(budget, max(0, max_tokens - 1024))`。
///
/// **未实现**：`compat.forceAdaptiveThinking` / `compat.supportsMidConvoEffort` 两条自适应分支
/// （ref `:1153-1159`、`:1165-1170`）——`models.json` 的 `compat` 在 pi-rs 里没有建模
/// （`ModelDefinition` 无该字段，只落在 `extra` 里）。
pub fn thinking_config_for_level(
    level: Option<&str>,
    base_max_tokens: Option<u64>,
    model_max_tokens: u64,
) -> Option<ThinkingConfig> {
    let level = level?;
    if level == "off" {
        return Some(ThinkingConfig::Disabled);
    }
    let (max_tokens, thinking_budget) =
        adjust_max_tokens_for_thinking(base_max_tokens, model_max_tokens, level)?;
    Some(ThinkingConfig::Enabled {
        budget_tokens: Some(thinking_budget.min(max_tokens.saturating_sub(MIN_ANSWER_TOKENS))),
        display: Some("summarized"),
    })
}

impl ThinkingConfig {
    /// 转成请求体里的 `thinking` 对象；`None` 表示不发该字段。
    pub fn to_json(self) -> Option<Value> {
        match self {
            Self::Enabled {
                budget_tokens,
                display,
            } => Some(json!({
                "type": "enabled",
                "budget_tokens": budget_tokens.unwrap_or(1024),
                "display": display.unwrap_or("summarized"),
            })),
            Self::Adaptive { display, effort } => {
                let mut value = json!({ "type": "adaptive" });
                if let Some(display) = display.or(Some("summarized")) {
                    value["display"] = json!(display);
                }
                if let Some(effort) = effort {
                    value["output_config"] = json!({ "effort": effort });
                }
                Some(value)
            }
            Self::Disabled => Some(json!({ "type": "disabled" })),
        }
    }
}

/// 同 [`build_messages_request`]，但可带 thinking 配置。
/// 与 [`build_messages_request`] 相同，但**工具取自 request.tools**（生产路径用这个）。
///
/// ref `anthropic-messages.ts:1122/1141`：有工具时 `params.tools = convertTools(...)`；
/// `:1139-1140` 的 `if (tools.length > 0)` 表示**没有工具就不发**该字段。
pub fn build_messages_request_with_tools(
    request: &ProviderRequest,
    thinking: Option<ThinkingConfig>,
) -> Value {
    let tools: Vec<ToolSpecAnthropic> = request
        .tools
        .iter()
        .map(|tool| ToolSpecAnthropic {
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: tool.parameters.clone(),
            eager_input_streaming: true,
        })
        .collect();
    build_messages_request_with_thinking(request, &tools, thinking)
}

pub fn build_messages_request_with_thinking(
    request: &ProviderRequest,
    tools: &[ToolSpecAnthropic],
    thinking: Option<ThinkingConfig>,
) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(request.model));
    body.insert(
        "messages".to_string(),
        Value::Array(request.context.to_anthropic_messages()),
    );
    body.insert(
        "max_tokens".to_string(),
        json!(request.max_completion_tokens),
    );
    body.insert("stream".to_string(), json!(true));
    if let Some(system) = &request.context.system {
        // oracle 实测：TS 的 system 块**带** cache_control（golden 的 S12-anthropic 里没有，
        // 那是场景没走 cacheRetention；两种形态都由本函数按 context 的开关决定）。
        let mut block = json!({ "type": "text", "text": system });
        if request.context.cache_control_system {
            block["cache_control"] = json!({ "type": "ephemeral" });
        }
        body.insert("system".to_string(), json!([block]));
    }
    if !tools.is_empty() {
        body.insert(
            "tools".to_string(),
            Value::Array(
                tools
                    .iter()
                    .enumerate()
                    .map(|(index, tool)| {
                        // ref `:1483-1490`：`name` / `description` / `eager_input_streaming` /
                        // `input_schema`，**`cache_control` 只加在最后一个工具上**
                        // （`:1489` 的 `cacheControl && index === tools.length - 1`）。
                        // 条件用的就是 `cacheControl` 本身（`:1114` 再按
                        // `supportsCacheControlOnTools ?? true` 决定是否给工具用）——
                        // 这里与 system 块共用 `context.cache_control_system`。
                        let mut value = json!({
                            "name": tool.name,
                            "description": tool.description,
                            "eager_input_streaming": tool.eager_input_streaming,
                            "input_schema": tool.input_schema,
                        });
                        if request.context.cache_control_system && index == tools.len() - 1 {
                            value["cache_control"] = json!({ "type": "ephemeral" });
                        }
                        value
                    })
                    .collect(),
            ),
        );
    }
    if let Some(value) = thinking.and_then(ThinkingConfig::to_json) {
        body.insert("thinking".to_string(), value);
    }
    Value::Object(body)
}

use futures::StreamExt as _;
use std::collections::VecDeque;

use tokio_util::sync::CancellationToken;

use crate::decoder::{Rates, StreamDecoder, ANTHROPIC_MESSAGES};
use crate::openai::{CONNECT_TIMEOUT, RESPONSE_HEADER_TIMEOUT};
use crate::provider::{BoxFuture, Provider, Stream};
use crate::{AssistantEvent, ProviderError};

/// Anthropic 的 API 版本头（SDK 固定值）。
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic 的认证头。
///
/// **取值已定位**：`@anthropic-ai/sdk` 把 `apiKey` 转成 `x-api-key` 头，
/// oracle 实测 TS 的 10 种上下文全部带 `x-api-key: …` 与 `anthropic-version: 2023-06-01`，
/// 且**不带** `anthropic-beta`（该头只在传了 `betas` 或 OAuth 模式时才追加）。
/// 形态仍可配置，以覆盖网关自定义（`cf-aig-authorization` 等）与本地 mock。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthHeader {
    /// `x-api-key: <key>`——SDK 的默认形态，也是默认值。
    #[default]
    ApiKey,
    /// `authorization: Bearer <key>`（`ANTHROPIC_AUTH_TOKEN` 分支走的就是这个）。
    Bearer,
    /// 不加认证头（本地 mock、自带 headers 的网关）。
    None,
}

impl AuthHeader {
    /// 请求头名称；[`AuthHeader::None`] 返回空串。
    pub fn header_name(self) -> &'static str {
        match self {
            Self::ApiKey => "x-api-key",
            Self::Bearer => "authorization",
            Self::None => "",
        }
    }

    /// 请求头的取值；[`AuthHeader::None`] 返回 `None`。
    /// 请求头的取值。空 key 返回 `None`——避免发出空的认证头。
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

/// `/v1/messages` 的流式客户端。
///
/// 与 [`crate::openai::ChatCompletionsClient`] 一样**不做重试**，重试由上层
/// [`crate::retry::RetryPolicy`] 驱动。
#[derive(Debug, Clone)]
pub struct AnthropicMessagesClient {
    base_url: String,
    rates: Rates,
    provider: String,
    auth: AuthHeader,
    api_key: String,
    /// 模型自带头（对应 TS 的 `model.headers`）。
    headers: Vec<(String, String)>,
    cancel: CancellationToken,
}

impl AnthropicMessagesClient {
    /// 以 base url（形如 `http://host:port/v1`）与费率构造。
    /// 以 base url（形如 `http://host:port/v1`）与费率构造。
    ///
    /// 不带 API key——此时 [`AuthHeader::ApiKey`] 也不会产出头（值为空），
    /// 适合本地 mock。
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

    /// 追加模型自带头。
    #[must_use]
    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    /// 当前模型自带头。
    pub fn extra_headers(&self) -> Vec<(String, String)> {
        self.headers.clone()
    }

    /// 从 `models.json` 解析出的模型构造。
    #[must_use]
    pub fn from_resolved(
        resolved: &crate::config::ResolvedModel,
        auth: AuthHeader,
        api_key: &str,
    ) -> Self {
        Self {
            base_url: resolved.base_url.clone(),
            rates: resolved.rates,
            provider: resolved.provider_name.clone(),
            auth,
            api_key: api_key.to_string(),
            headers: Vec::new(),
            cancel: CancellationToken::new(),
        }
    }

    /// 给**解码器**装一个取消令牌（P4-15 步骤 4.2 的结论）。
    ///
    /// **生产路径不使用它**：abort 由 pi-core 的 `tokio::select!`（`crates/pi-core/src/agent/run.rs`
    /// 的"等下一个事件 vs cancel.cancelled()"）负责——abort 时它停止取事件并返回，
    /// 于是**流被丢弃**，reqwest 随之关闭连接、在途传输结束。
    /// 这个接口的用途是：① 嵌入式调用方自己持有令牌时可以直接取消解码；② 测试用它覆盖
    /// 解码器的取消分支（`crates/pi-ai/tests/t2_5_cancel.rs`、`t2_16_abort_mid_stream.rs` 等）。
    /// 行为与 ref 等价：ref 是把 `AbortSignal` 透传给 fetch（效果同为"请求不再继续"）。
    /// 指定认证头形态。
    #[must_use]
    pub fn with_auth(mut self, auth: AuthHeader) -> Self {
        self.auth = auth;
        self
    }

    /// 指定 key（认证头形态沿用当前 `auth`，默认 `x-api-key`）。
    #[must_use]
    pub fn with_api_key(mut self, api_key: &str) -> Self {
        self.api_key = api_key.to_string();
        self
    }

    /// 绑定取消令牌。
    #[must_use]
    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = token;
        self
    }

    /// 请求端点（供测试断言）。
    pub fn endpoint_for_test(&self) -> String {
        self.endpoint()
    }

    /// 当前 base url。
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 当前 provider 名。
    pub fn provider_name(&self) -> &str {
        &self.provider
    }

    /// 端点固定带 `beta=true`。
    ///
    /// 依据：`@anthropic-ai/sdk/resources/beta/messages/messages.js:43` 写死
    /// `POST /v1/messages?beta=true`（摘录见 `fixtures/vendor-evidence/`）。
    /// 已裁定：与 TS 保持一致。
    fn endpoint(&self) -> String {
        messages_endpoint(&self.base_url)
    }
}

impl Provider for AnthropicMessagesClient {
    fn stream<'a>(
        &'a self,
        request: ProviderRequest,
    ) -> BoxFuture<'a, Result<Stream, ProviderError>> {
        Box::pin(async move {
            // ref `:1122/:1141`：**有工具就要发**（此前这里硬编码 `&[]`，导致请求体缺 `tools`）。
            let body = build_messages_request_with_tools(&request, request.thinking);
            // 同 openai 侧：不设总超时，只对"等到响应头"计时。
            let http: reqwest::Client = reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .map_err(|error| {
                    ProviderError::transport(ProviderError::describe_transport_error(&error))
                })?;
            let mut builder = http
                .post(self.endpoint())
                .header("anthropic-version", ANTHROPIC_VERSION)
                .json(&body);
            if let Some((name, value)) = self.auth.header_value(&self.api_key) {
                builder = builder.header(name, value);
            }
            // 请求级头优先于客户端级头（TS 里 options.headers 覆盖 defaultHeaders），
            // 并按 ref `:1074` 把非空的 beta 特性变成 `anthropic-beta` 头。
            let beta = beta_features(&request, &self.headers);
            let headers = build_request_headers(&self.headers, &request.headers, &beta);
            for (name, value) in &headers {
                builder = builder.header(name, value);
            }
            let response = tokio::time::timeout(RESPONSE_HEADER_TIMEOUT, builder.send())
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

            let decoder = StreamDecoder::new(
                ANTHROPIC_MESSAGES,
                &self.provider,
                &request.model,
                self.rates,
            )
            .map_err(|error| ProviderError::stream_error(error.to_string()))?
            .with_cancel(self.cancel.clone());
            let bytes = response.bytes_stream();

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
                                        // 取消不是错误事件：TS 只把助手消息标成 aborted。
                                        // 最终消息仍要带出去（见 openai 侧同处的说明）。
                                        store_final(&slot, &decoder);
                                        break 'stream;
                                    }
                                    Err(error) => {
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
