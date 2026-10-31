//! provider 流解码：把 SSE 字节流变成 [`AssistantEvent`] 序列与最终 [`AssistantMessage`]。
//!
//! 事件序与 usage/cost 的算法必须与 TS 版 pi-ai 一致，判据是
//! `fixtures/expected/*.expected.json`（由 `tools/oracle` 用 npm 包里的 pi-ai 生成）。
//!
//! 一个解码器同时支持 `openai-completions` 与 `anthropic-messages`，差异集中在 `handle` 的分支里；
//! 两者共享"内容块累加器"：文本与 thinking 按块累积，tool call 的参数按原始 JSON 字符串累积。

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::sse::{SseError, SseParser};
use crate::types::{AssistantEvent, AssistantMessage, Content, Cost, StopReason, Usage};

/// 支持的两种 api 标识。
pub const OPENAI_COMPLETIONS: &str = "openai-completions";
/// 支持的两种 api 标识。
pub const ANTHROPIC_MESSAGES: &str = "anthropic-messages";

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error(transparent)]
    Sse(#[from] SseError),
    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("不支持的 api: {0}")]
    UnsupportedApi(String),
    /// 取消令牌已触发：解码停止，后续不再产出事件。
    #[error("已取消")]
    Aborted,
}

/// 每百万 token 的费率，与 oracle 使用的模型对象一致。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Rates {
    /// 全零费率：mock 场景与只关心内容不关心价格的测试用。
    pub fn zero() -> Self {
        Self {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        }
    }
}

/// `calculateContextTokens` 的口径（`ref/pi-0.87.1/packages/ai/src/utils/estimate.ts`）。
///
/// `totalTokens` 非 0 时直接用；否则退回 `input + output + cacheRead + cacheWrite`。
/// 这里的 `||` 语义与 TS 一致——0 视为「没有」。
pub fn calculate_context_tokens(usage: &Usage) -> u64 {
    if usage.total_tokens != 0 {
        return usage.total_tokens;
    }
    usage.input + usage.output + usage.cache_read + usage.cache_write
}

/// 按 `calculateCost` 的运算顺序折算费用（`ref/pi-0.87.1/packages/ai/src/models.ts`）。
///
/// 运算顺序逐字对应 TS：先除 1_000_000 再乘 token；cache 写入按 1h/短保留分开，
/// 1h 部分按基础 input 单价的两倍计；total 是四项**按 input、output、cacheRead、cacheWrite 顺序**相加。
/// 顺序不能调换：同样的数字换个次序会得到不同的 f64。
pub fn calculate_cost(rates: Rates, usage: &Usage) -> Cost {
    let long_write = usage.cache_write_1h.unwrap_or(0) as f64;
    let short_write = usage.cache_write as f64 - long_write;
    let cost_input = rates.input / 1_000_000.0 * usage.input as f64;
    let cost_output = rates.output / 1_000_000.0 * usage.output as f64;
    let cost_cache_read = rates.cache_read / 1_000_000.0 * usage.cache_read as f64;
    let cost_cache_write =
        (rates.cache_write * short_write + rates.input * 2.0 * long_write) / 1_000_000.0;
    Cost {
        input: cost_input,
        output: cost_output,
        cache_read: cost_cache_read,
        cache_write: cost_cache_write,
        total: cost_input + cost_output + cost_cache_read + cost_cache_write,
    }
}

#[derive(Debug, Clone)]
enum Block {
    Text(String),
    /// **必须带 provider 记下的 signature**（ref `openai-completions.ts:619-623`：命中的 reasoning 字段名）。
    Thinking {
        text: String,
        signature: String,
        /// ref `:655` 的 `redacted: true`（只有 `redacted_thinking` 块为真）。
        redacted: bool,
    },
    ToolCall {
        id: String,
        name: String,
        args: String,
    },
}

/// 增量解码器。
#[derive(Debug)]
pub struct StreamDecoder {
    api: String,
    provider: String,
    model: String,
    rates: Rates,
    sse: SseParser,
    blocks: Vec<Block>,
    started: bool,
    finished: bool,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cache_write_1h: Option<u64>,
    reasoning: Option<u64>,
    tool_slots: std::collections::BTreeMap<usize, usize>,
    stop_reason: StopReason,
    raw_stop_reason: Option<String>,
    end_turn: Option<bool>,
    error_message: Option<String>,
    /// provider 响应标识（首个 chunk 的 `id`；照 ref `api/openai-completions.ts:558`）。
    response_id: Option<String>,
    cancel: Option<CancellationToken>,
}

impl StreamDecoder {
    /// 创建一个解码器。`api` 必须是 [`OPENAI_COMPLETIONS`] 或 [`ANTHROPIC_MESSAGES`]。
    pub fn new(api: &str, provider: &str, model: &str, rates: Rates) -> Result<Self, DecodeError> {
        if api != OPENAI_COMPLETIONS && api != ANTHROPIC_MESSAGES {
            return Err(DecodeError::UnsupportedApi(api.to_string()));
        }
        Ok(Self {
            api: api.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            rates,
            sse: SseParser::new(),
            blocks: Vec::new(),
            started: false,
            finished: false,
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: None,
            tool_slots: std::collections::BTreeMap::new(),
            reasoning: None,
            stop_reason: StopReason::Pending,
            raw_stop_reason: None,
            end_turn: None,
            error_message: None,
            response_id: None,
            cancel: None,
        })
    }

    /// 绑定取消令牌。被取消后不再产生任何事件（V3：流式中途与事件之间都要能停）。
    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// 当前是否已被取消。
    pub fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }

    /// 喂入一段字节（可任意切分），返回本次产生的事件。
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<AssistantEvent>, DecodeError> {
        if self.is_cancelled() {
            return Err(self.abort());
        }
        let parsed = self.sse.push(bytes)?;
        let mut out = Vec::new();
        for event in parsed {
            if self.is_cancelled() {
                self.mark_aborted();
                return Err(DecodeError::Aborted);
            }
            self.handle(event, &mut out)?;
            if self.finished {
                break;
            }
        }
        Ok(out)
    }

    /// 报告取消；已经拿到确定停止原因的消息保持原样。
    fn abort(&mut self) -> DecodeError {
        self.mark_aborted();
        DecodeError::Aborted
    }

    /// 只改状态，不产生事件。已有确定停止原因时不覆盖。
    fn mark_aborted(&mut self) {
        self.finished = true;
        if self.stop_reason == StopReason::Pending {
            self.stop_reason = StopReason::Aborted;
            if self.error_message.is_none() {
                self.error_message = Some("Request aborted".to_string());
            }
        }
    }

    /// 流结束：处理残余事件并收尾未闭合的块。
    pub fn finish(&mut self) -> Result<Vec<AssistantEvent>, DecodeError> {
        let mut out = Vec::new();
        // 取消优先于收尾：被取消的流不该被当成"缺少 finish_reason"的错误流。
        // 已经拿到确定停止原因（如 finish_reason 先到）的消息保持原样。
        if self.is_cancelled() {
            self.mark_aborted();
        }
        if !self.finished {
            let last = self.sse.finish()?;
            if let Some(event) = last {
                self.handle(event, &mut out)?;
            }
        }
        if !self.finished {
            self.finished = true;
            if self.stop_reason == StopReason::Pending {
                self.stop_reason = StopReason::Error;
                self.error_message
                    .get_or_insert_with(|| "Stream ended without finish_reason".to_string());
            }
            self.close_open_blocks(&mut out);
        }
        Ok(out)
    }

    /// 最终助手消息快照。
    pub fn final_message(&self) -> AssistantMessage {
        let content = self
            .blocks
            .iter()
            .map(|block| match block {
                Block::Text(text) => Content::Text {
                    text: text.clone(),
                    text_signature: None,
                },
                Block::Thinking {
                    text,
                    signature,
                    redacted,
                } => Content::Thinking {
                    thinking: text.clone(),
                    thinking_signature: if signature.is_empty() {
                        None
                    } else {
                        Some(signature.clone())
                    },
                    redacted: if *redacted { Some(true) } else { None },
                },
                Block::ToolCall { id, name, args } => Content::ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: parse_arguments(args),
                    thought_signature: None,
                },
            })
            .collect();
        // **usage 只在流结束后才进快照**：TS 侧部分消息的 usage 全为 0，要等 `done` 事件
        // （OpenAI 的 `stream_options.include_usage` 把 usage 放在最后一个 chunk）才填。
        // 我原先在流式过程中就带上累积用量，`golden/S01` 的 `message_update` 里
        // usage 是 0 而我是 10/2/12，由 B 层逐层比对指出。
        // **不做"未 finished 就清零"的门控**：我的解码器一次性解完一个 chunk，
        // 进循环时 `finished` 已为真，这类门控要么无效、要么把**终态**也清零
        // （我试过，`agent_end` 里 assistant 的 usage 因此变成 0，被 B 层比对指出）。
        // 流式快照的清零改由"发 `message_update` 时按事件判断"负责，职责分离更清楚。
        let (input, output, cache_read, cache_write, cache_write_1h, reasoning) = (
            self.input,
            self.output,
            self.cache_read,
            self.cache_write,
            self.cache_write_1h,
            self.reasoning,
        );
        let mut usage = Usage {
            input,
            output,
            cache_read,
            cache_write,
            cache_write_1h,
            reasoning,
            // 不采信 provider 给的 total_tokens：一律自行求和（TS 同款）。
            total_tokens: input + output + cache_read + cache_write,
            cost: Cost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
                total: 0.0,
            },
        };
        usage.cost = calculate_cost(self.rates, &usage);
        AssistantMessage {
            role: "assistant".to_string(),
            content,
            api: self.api.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            response_model: None,
            usage,
            stop_reason: self.stop_reason,
            error_message: self.error_message.clone(),
            raw_stop_reason: self.raw_stop_reason.clone(),
            end_turn: self.end_turn,
            response_id: self.response_id.clone(),
            timestamp: 0,
        }
    }

    /// 取某个 tool call 块在当前时刻的拼装结果（id、name、已解析参数）。
    pub fn tool_call_at(
        &self,
        index: usize,
    ) -> Option<(String, String, serde_json::Map<String, Value>)> {
        match self.blocks.get(index) {
            Some(Block::ToolCall { id, name, args }) => {
                Some((id.clone(), name.clone(), parse_arguments(args)))
            }
            _ => None,
        }
    }

    /// 当前累积的内容块数。
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    fn close_open_blocks(&mut self, out: &mut Vec<AssistantEvent>) {
        for index in 0..self.blocks.len() {
            match &self.blocks[index] {
                Block::Text(text) => out.push(AssistantEvent::TextEnd {
                    content_index: index,
                    content: text.clone(),
                }),
                Block::Thinking { text, .. } => out.push(AssistantEvent::ThinkingEnd {
                    content_index: index,
                    content: text.clone(),
                }),
                Block::ToolCall { .. } => out.push(AssistantEvent::ToolCallEnd {
                    content_index: index,
                }),
            }
        }
    }

    fn handle(
        &mut self,
        event: crate::sse::SseEvent,
        out: &mut Vec<AssistantEvent>,
    ) -> Result<(), DecodeError> {
        let data = event.data.trim();
        if data.is_empty() {
            return Ok(());
        }
        if data == "[DONE]" {
            self.close_open_blocks(out);
            if self.stop_reason == StopReason::Pending {
                self.stop_reason = StopReason::Stop;
            }
            self.finished = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(data)?;
        if self.api == OPENAI_COMPLETIONS {
            self.handle_openai(&value, out);
        } else {
            self.handle_anthropic(&value, out);
        }
        Ok(())
    }

    // ---------------- OpenAI Chat Completions ----------------

    fn handle_openai(&mut self, value: &Value, out: &mut Vec<AssistantEvent>) {
        // provider 响应标识：取**首个** chunk 的顶层 `id`（ref `packages/ai/src/api/openai-completions.ts:558`
        // 的 `output.responseId ||= chunk.id;`；`||=` 语义 ⇒ 只在还没值时写）。
        if self.response_id.is_none() {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                if !id.is_empty() {
                    self.response_id = Some(id.to_string());
                }
            }
        }
        if let Some(err) = value.get("error") {
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("stream error")
                .to_string();
            self.error_message = Some(message.clone());
            self.stop_reason = StopReason::Error;
            self.close_open_blocks(out);
            out.push(AssistantEvent::Error {
                reason: StopReason::Error,
                message,
            });
            self.finished = true;
            return;
        }

        if !self.started {
            self.started = true;
            out.push(AssistantEvent::Start);
        }
        if let Some(usage) = value.get("usage").and_then(Value::as_object) {
            let get_u = |key: &str| usage.get(key).and_then(Value::as_u64);
            // 缓存口径逐字照 `api/openai-completions.ts:1522-1539`：
            // cacheRead 依次取 prompt_tokens_details.cached_tokens → prompt_cache_hit_tokens → cached_tokens；
            // cacheWrite 取 prompt_tokens_details.cache_write_tokens。
            // input 是 prompt_tokens 扣掉两份缓存后的余量（下限 0），不是 prompt_tokens 本身。
            let details = usage
                .get("prompt_tokens_details")
                .and_then(Value::as_object);
            let cache_read = details
                .and_then(|d| d.get("cached_tokens"))
                .or_else(|| usage.get("prompt_cache_hit_tokens"))
                .or_else(|| usage.get("cached_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let cache_write = details
                .and_then(|d| d.get("cache_write_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            self.cache_read = cache_read;
            self.cache_write = cache_write;
            let prompt_tokens = get_u("prompt_tokens").unwrap_or(0);
            self.input = prompt_tokens.saturating_sub(cache_read + cache_write);
            if let Some(v) = get_u("completion_tokens") {
                self.output = v;
            }
            let details = usage
                .get("completion_tokens_details")
                .and_then(Value::as_object);
            let reasoning = details
                .and_then(|d| d.get("reasoning_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            self.reasoning = Some(reasoning);
        }

        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return;
        };
        let delta = choice.get("delta").cloned().unwrap_or(Value::Null);
        // ref `openai-completions.ts:605-623`：按 **reasoning_content → reasoning → reasoning_text** 取
        // **第一个非空**字段，并把**命中的字段名**当作该 thinking 块的 signature
        // （特例：provider 是 `opencode-go` 且命中 `reasoning` 时记作 `reasoning_content`）。
        for field in ["reasoning_content", "reasoning", "reasoning_text"] {
            let Some(value) = delta.get(field).and_then(Value::as_str) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            let signature = if self.provider == "opencode-go" && field == "reasoning" {
                "reasoning_content"
            } else {
                field
            };
            self.push_thinking(value, signature, out);
            break;
        }
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                self.push_text(content, out);
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                self.push_openai_tool_call(call, out);
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.raw_stop_reason = Some(reason.to_string());
            self.stop_reason = map_finish_reason(reason);
            self.close_open_blocks(out);
            out.push(AssistantEvent::Done {
                reason: self.stop_reason,
            });
            self.finished = true;
        }
    }

    fn push_text(&mut self, content: &str, out: &mut Vec<AssistantEvent>) {
        if let Some(Block::Text(text)) = self.blocks.last_mut() {
            text.push_str(content);
            let index = self.blocks.len() - 1;
            out.push(AssistantEvent::TextDelta {
                content_index: index,
                delta: content.to_string(),
            });
            return;
        }
        let index = self.blocks.len();
        self.blocks.push(Block::Text(content.to_string()));
        out.push(AssistantEvent::TextStart {
            content_index: index,
        });
        out.push(AssistantEvent::TextDelta {
            content_index: index,
            delta: content.to_string(),
        });
    }

    fn push_thinking(&mut self, reasoning: &str, signature: &str, out: &mut Vec<AssistantEvent>) {
        if let Some(Block::Thinking {
            text,
            signature: existing,
            ..
        }) = self.blocks.last_mut()
        {
            // ref 的 `ensureThinkingBlock(signature)`：**签名不同就要新开块**。
            if *existing == signature {
                text.push_str(reasoning);
                let index = self.blocks.len() - 1;
                out.push(AssistantEvent::ThinkingDelta {
                    content_index: index,
                    delta: reasoning.to_string(),
                });
                return;
            }
        }
        let index = self.blocks.len();
        self.blocks.push(Block::Thinking {
            text: reasoning.to_string(),
            signature: signature.to_string(),
            redacted: false,
        });
        out.push(AssistantEvent::ThinkingStart {
            content_index: index,
        });
        out.push(AssistantEvent::ThinkingDelta {
            content_index: index,
            delta: reasoning.to_string(),
        });
    }

    fn push_openai_tool_call(&mut self, call: &Value, out: &mut Vec<AssistantEvent>) {
        // SSE 里的 `index` 是**工具序号**，不是内容块序号：thinking 之后出现的
        // 第一个 tool call 往往也是 index 0，直接拿它当内容块下标会撞上 thinking 块。
        // 这里维护工具序号 → 内容块序号的映射，首次出现时按到达顺序追加新块。
        let tool_index = call
            .get("index")
            .and_then(Value::as_u64)
            .map(|i| i as usize)
            .unwrap_or(0);
        let index = match self.tool_slots.get(&tool_index) {
            Some(existing) => *existing,
            None => {
                self.blocks.push(Block::ToolCall {
                    id: String::new(),
                    name: String::new(),
                    args: String::new(),
                });
                let allocated = self.blocks.len() - 1;
                self.tool_slots.insert(tool_index, allocated);
                allocated
            }
        };
        let id = call.get("id").and_then(Value::as_str).map(str::to_string);
        let name = call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let arguments = call
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .map(str::to_string);

        let mut opened = false;
        if let Some(Block::ToolCall {
            id: slot_id,
            name: slot_name,
            ..
        }) = self.blocks.get_mut(index)
        {
            if let Some(id) = id {
                *slot_id = id;
                opened = true;
            }
            if let Some(name) = name {
                *slot_name = name;
                opened = true;
            }
        }
        if opened {
            out.push(AssistantEvent::ToolCallStart {
                content_index: index,
            });
        }
        if let Some(arguments) = arguments {
            if let Some(Block::ToolCall { args, .. }) = self.blocks.get_mut(index) {
                args.push_str(&arguments);
            }
            out.push(AssistantEvent::ToolCallDelta {
                content_index: index,
                delta: arguments,
            });
        }
    }

    // ---------------- Anthropic Messages ----------------

    fn handle_anthropic(&mut self, value: &Value, out: &mut Vec<AssistantEvent>) {
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "message_start" => {
                self.started = true;
                out.push(AssistantEvent::Start);
                // usage 字段映射照 `api/anthropic-messages.ts:615-625`：
                // 五项都是 `… || 0`——**缺失时赋 0**（不是"保持上一次的值"），
                // 其中 `cache_creation?.ephemeral_1h_input_tokens` 还要过一次可选链（`:621`）。
                // 这条 `|| 0` 很关键：`cacheWrite1h` 只有显式赋 0 才会被序列化出来
                // （`types.rs:35` 的 `skip_serializing_if`），golden/S12-anthropic 记的就是 `"cacheWrite1h":0`。
                // totalTokens 由四项求和（`:623-624`，见 `final_message`）。
                let usage = value.pointer("/message/usage");
                let number = |key: &str| {
                    usage
                        .and_then(|usage| usage.get(key))
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                };
                self.input = number("input_tokens");
                self.output = number("output_tokens");
                self.cache_read = number("cache_read_input_tokens");
                self.cache_write = number("cache_creation_input_tokens");
                self.cache_write_1h = Some(
                    usage
                        .and_then(|usage| {
                            usage.pointer("/cache_creation/ephemeral_1h_input_tokens")
                        })
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                );
            }
            "ping" => {}
            "content_block_start" => {
                let index = value
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|i| i as usize)
                    .unwrap_or(self.blocks.len());
                let block = value.get("content_block").cloned().unwrap_or(Value::Null);
                let block_type = block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                while self.blocks.len() <= index {
                    self.blocks.push(Block::Text(String::new()));
                }
                if block_type == "thinking" || block_type == "redacted_thinking" {
                    // ref `:641-649`（thinking）与 `:650-659`（redacted_thinking）
                    let redacted = block_type == "redacted_thinking";
                    let text = if redacted {
                        "[Reasoning redacted]".to_string() // ref `:653`
                    } else {
                        block
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    };
                    // ref `:645` 的 `?? ""` 与 `:654` 的 `data`
                    let signature = block
                        .get(if redacted { "data" } else { "signature" })
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.blocks[index] = Block::Thinking {
                        text,
                        signature,
                        redacted,
                    };
                    out.push(AssistantEvent::ThinkingStart {
                        content_index: index,
                    });
                } else if block_type == "tool_use" {
                    let id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.blocks[index] = Block::ToolCall {
                        id,
                        name,
                        args: String::new(),
                    };
                    out.push(AssistantEvent::ToolCallStart {
                        content_index: index,
                    });
                } else {
                    let text = block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.blocks[index] = Block::Text(text);
                    out.push(AssistantEvent::TextStart {
                        content_index: index,
                    });
                }
            }
            "content_block_delta" => {
                let index = value
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|i| i as usize)
                    .unwrap_or_default();
                let delta = value.get("delta").cloned().unwrap_or(Value::Null);
                let delta_type = delta
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                match delta_type.as_str() {
                    "text_delta" => {
                        let text = delta
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(Block::Text(buffer)) = self.blocks.get_mut(index) {
                            buffer.push_str(text);
                        }
                        out.push(AssistantEvent::TextDelta {
                            content_index: index,
                            delta: text.to_string(),
                        });
                    }
                    "signature_delta" => {
                        // ref `:712-717`：`|| ""` 再 `+=`；**不发事件**。
                        let signature = delta
                            .get("signature")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(Block::Thinking {
                            signature: current, ..
                        }) = self.blocks.get_mut(index)
                        {
                            current.push_str(signature);
                        }
                    }
                    "thinking_delta" => {
                        let text = delta
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(Block::Thinking { text: buffer, .. }) =
                            self.blocks.get_mut(index)
                        {
                            buffer.push_str(text);
                        }
                        out.push(AssistantEvent::ThinkingDelta {
                            content_index: index,
                            delta: text.to_string(),
                        });
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(Block::ToolCall { args, .. }) = self.blocks.get_mut(index) {
                            args.push_str(partial);
                        }
                        out.push(AssistantEvent::ToolCallDelta {
                            content_index: index,
                            delta: partial.to_string(),
                        });
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = value
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|i| i as usize)
                    .unwrap_or_default();
                match self.blocks.get(index) {
                    Some(Block::Text(text)) => out.push(AssistantEvent::TextEnd {
                        content_index: index,
                        content: text.clone(),
                    }),
                    Some(Block::Thinking { text, .. }) => out.push(AssistantEvent::ThinkingEnd {
                        content_index: index,
                        content: text.clone(),
                    }),
                    Some(Block::ToolCall { .. }) => out.push(AssistantEvent::ToolCallEnd {
                        content_index: index,
                    }),
                    None => {}
                }
            }
            "message_delta" => {
                // 代理可能只在 message_delta 里给 usage，字段缺失时保留 message_start 的初值。
                if let Some(output) = value
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64)
                {
                    self.output = output;
                }
                if let Some(input) = value.pointer("/usage/input_tokens").and_then(Value::as_u64) {
                    self.input = input;
                }
                if let Some(cache_read) = value
                    .pointer("/usage/cache_read_input_tokens")
                    .and_then(Value::as_u64)
                {
                    self.cache_read = cache_read;
                }
                if let Some(cache_write) = value
                    .pointer("/usage/cache_creation_input_tokens")
                    .and_then(Value::as_u64)
                {
                    self.cache_write = cache_write;
                }
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.raw_stop_reason = Some(reason.to_string());
                    self.stop_reason = map_anthropic_stop(reason);
                }
            }
            "message_stop" => {
                if self.stop_reason == StopReason::Pending {
                    self.stop_reason = StopReason::Stop;
                }
                out.push(AssistantEvent::Done {
                    reason: self.stop_reason,
                });
                self.finished = true;
            }
            "error" => {
                self.error_message = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| Some("anthropic stream error".to_string()));
                self.stop_reason = StopReason::Error;
                let message = self
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "anthropic stream error".to_string());
                out.push(AssistantEvent::Error {
                    reason: StopReason::Error,
                    message,
                });
                self.finished = true;
            }
            _ => {}
        }
    }
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::Stop,
        "length" => StopReason::Length,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        _ => StopReason::Error,
    }
}

fn map_anthropic_stop(reason: &str) -> StopReason {
    match reason {
        "end_turn" | "stop_sequence" => StopReason::Stop,
        "max_tokens" => StopReason::Length,
        "tool_use" => StopReason::ToolUse,
        _ => StopReason::Error,
    }
}

/// tool call 参数的 JSON 解析；空串按 `{}` 处理，非法 JSON 视为 `{}`。
fn parse_arguments(args: &str) -> serde_json::Map<String, Value> {
    if args.trim().is_empty() {
        return serde_json::Map::new();
    }
    match serde_json::from_str::<Value>(args) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}
