//! Context 与消息转换（纯函数，无 I/O）。
//!
//! 对应 RUNBOOK B1 的"trait 入参为 Context"与 B2 的"消息转换实现为纯函数"。
//! 形状取自 `golden/S25-anthropic` 与 `golden/S02` 的实际请求体，
//! 规则来源见 `spec/a-types.md`（消息类型）与 `spec/b-agent-loop.md`（转换规则）。
//!
//! **尚未实现**（见 `notes/p2-signoff.md`）：孤儿 tool call 的丢弃规则、
//! error/aborted 的 assistant 消息丢弃、thinking 回放、reasoning 兼容项。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 一次请求需要的全部消息上下文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Context {
    /// 系统提示；`None` 时请求体不出现 system 字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// **会话消息**（P3.0：单一数据源）。请求体形状由 [`Context::legacy_messages`]
    /// 在转换时派生，不再作为存储形态。
    #[serde(default)]
    pub messages: Vec<crate::message::Message>,
    /// 是否给**最后一条 user 文本块**加 `cache_control: ephemeral`。
    ///
    /// golden 里 Anthropic 侧只有最后一轮带该标记；openai 侧没有这个概念。
    #[serde(default = "default_true")]
    pub cache_control_last_user: bool,
    /// OpenAI 侧 system 消息的角色：`None` = `system`，推理模型用 `developer`
    /// （`openai-completions.ts:1225`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_role: Option<String>,
    /// OpenAI 侧的 `reasoning_effort`（思考等级，`openai-completions.ts:925-928`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// system 块是否带 `cache_control`。oracle 实测 TS 侧恒为 true。
    #[serde(default = "default_true")]
    pub cache_control_system: bool,
}

fn default_true() -> bool {
    true
}

/// 手写 `Default`：`cache_control_last_user` 的默认值是 true，而 `bool::default()` 是 false——
/// 只靠 serde 的 `default = "…"` 不够，代码里直接构造时也会走 `Default`。
impl Default for Context {
    fn default() -> Self {
        Self {
            system: None,
            messages: Vec::new(),
            cache_control_last_user: true,
            cache_control_system: true,
            instruction_role: None,
            reasoning_effort: None,
        }
    }
}

impl Context {
    /// 转换成 `anthropic-messages` 的 messages 数组。
    ///
    /// 规则（逐条对应 golden/S25-anthropic 第 3 次请求）：
    /// - `Text` → `{type:"text", text}`；`Image` → `{type:"image", source:{type:"base64", …}}`；
    /// - assistant 的 `ToolUse` → `{type:"tool_use", id, name, input}`；
    /// - user 的 `ToolResult` → `{type:"tool_result", tool_use_id, content, is_error}`，
    ///   其 content 是**块数组**（含 image）；
    /// - 最后一条 user 消息的文本块加 `cache_control: {type:"ephemeral"}`。
    pub fn to_anthropic_messages(&self) -> Vec<Value> {
        // **直接消费 `Message`**（P3.0 收尾）：不再经 `ContextMessage`/`ContextBlock`。
        let messages = crate::message::transform_messages(&self.messages);
        let last_user = messages.iter().rposition(is_user_role);
        messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                let (role, mut blocks) = message_to_anthropic(message)?;
                // ref `:1331`：空 thinking 且无签名 → 跳过该块（上面用 `Value::Null` 标记）。
                blocks.retain(|block| !block.is_null());
                // oracle 实测：cache_control 落在**最后一条 user 消息的最后一块**上，
                // 与该块是 text 还是 image 无关（with-image 形态证实）。
                if self.cache_control_last_user && Some(index) == last_user && !blocks.is_empty() {
                    if let Some(last) = blocks.last_mut() {
                        last["cache_control"] = json!({ "type": "ephemeral" });
                    }
                }
                Some(json!({ "role": role, "content": blocks }))
            })
            .collect()
    }

    /// 转换成 `openai-completions` 的 messages 数组。
    ///
    /// 规则（对应 golden/S02 与 S25-openai）：
    /// - system 单独成一条 `{role:"system", content:"…"}`（字符串，不是块数组）；
    /// - user/assistant 的文本与图片合成**一个**块数组；
    /// - `ToolResult` 拆成独立的 `{role:"tool", tool_call_id, content}` 消息，
    ///   content 是拼接后的文本（OpenAI 的 tool 消息没有块结构）。
    pub fn to_openai_messages(&self) -> Vec<Value> {
        self.to_openai_messages_impl("")
    }

    /// 与 `to_openai_messages` 相同，但知道 **provider**——ref 的 reasoning 字段名要看它
    /// （`openai-completions.ts:1335-1337` 的 opencode-go 特例）。
    pub fn to_openai_messages_with_provider(&self, provider: &str) -> Vec<Value> {
        self.to_openai_messages_impl(provider)
    }

    /// 发送侧按 `thinkingSignature` 回填 reasoning 字段（ref `openai-completions.ts:1331-1342`）：
    /// `:1334` 取第一个非空 thinking 块的 signature → `:1335-1337` opencode-go 特例 →
    /// `:1338-1340` 只在 signature 属于白名单（`:268-274`）时写 `assistantMsg[signature]`；
    /// 值是**全部非空 thinking 块用换行连接**；signature 为空/缺失则**不写任何字段**。
    fn reasoning_field_for(provider: &str, block: &crate::Content) -> Option<(String, String)> {
        let crate::Content::Thinking {
            thinking,
            thinking_signature,
            ..
        } = block
        else {
            return None;
        };
        if thinking.is_empty() {
            return None;
        }
        let signature = thinking_signature.clone()?;
        let signature = if provider == "opencode-go" && signature == "reasoning" {
            "reasoning_content".to_string()
        } else {
            signature
        };
        if !matches!(
            signature.as_str(),
            "reasoning" | "reasoning_content" | "reasoning_text"
        ) {
            return None;
        }
        Some((signature, thinking.clone()))
    }

    /// 把 reasoning 字段挂到已构造好的 assistant 消息上（ref `openai-completions.ts:1331-1342`）。
    fn attach_reasoning(message: &mut Value, provider: &str, assistant: &crate::AssistantMessage) {
        let thinking: Vec<(String, String)> = assistant
            .content
            .iter()
            .filter_map(|block| Self::reasoning_field_for(provider, block))
            .collect();
        if thinking.is_empty() {
            return;
        }
        let field = thinking[0].0.clone();
        let value = thinking
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        message[field] = Value::String(value);
    }

    fn to_openai_messages_impl(&self, provider: &str) -> Vec<Value> {
        // **直接消费 `Message`**（P3.0 收尾）。
        use crate::message::Message;
        let messages = crate::message::transform_messages(&self.messages);
        let mut out: Vec<Value> = Vec::new();
        if let Some(system) = &self.system {
            out.push(json!({
                "role": self.instruction_role.as_deref().unwrap_or("system"),
                "content": system,
            }));
        }
        for message in &messages {
            match message {
                // system 由 `self.system` 承载（与 Anthropic 侧同样处理）。
                Message::System { .. } => {}
                Message::ToolResult {
                    tool_call_id,
                    content,
                    ..
                } => {
                    // ref `api/openai-completions.ts:1398-1420`：tool 消息只带**文本**；
                    // 没有文本块但有图片时用 `(see attached image)` 占位。
                    let text_blocks: Vec<String> = content
                        .iter()
                        .filter_map(|inner| match inner {
                            crate::message::ToolResultContent::Text { text, .. } => {
                                Some(text.clone())
                            }
                            crate::message::ToolResultContent::Image { .. } => None,
                        })
                        .collect();
                    let image_blocks: Vec<&crate::message::ToolResultContent> = content
                        .iter()
                        .filter(|inner| {
                            matches!(inner, crate::message::ToolResultContent::Image { .. })
                        })
                        .collect();
                    let text = if !text_blocks.is_empty() {
                        text_blocks.join("\n")
                    } else if !image_blocks.is_empty() {
                        String::from("(see attached image)")
                    } else {
                        String::new()
                    };
                    out.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": text,
                    }));
                    // ref `:1426/1442-1460`：有图片且模型支持图片输入时，另推一条 user 消息
                    // （`compat.requiresAssistantAfterToolResult` 为 false 时不推 assistant）。
                    // **已知限制**：pi-rs 未建模 `model.input` 能力（`types.rs:30` 的 `input`
                    // 是 token 用量）⇒ 这里按 S25 所用 mock 模型的声明（支持 image）处理。
                    if !image_blocks.is_empty() {
                        let mut blocks: Vec<serde_json::Value> = vec![json!({
                            "type": "text",
                            "text": "Attached image(s) from tool result:",
                        })];
                        for block in &image_blocks {
                            if let crate::message::ToolResultContent::Image {
                                data,
                                mime_type,
                                ..
                            } = block
                            {
                                blocks.push(json!({
                                    "type": "image_url",
                                    "image_url": {
                                        "url": format!("data:{mime_type};base64,{data}"),
                                    },
                                }));
                            }
                        }
                        out.push(json!({ "role": "user", "content": blocks }));
                    }
                }
                Message::Assistant(assistant) => {
                    // **同一 assistant 消息的全部 tool_call 合成一条消息**（一个 `tool_calls` 数组）。
                    // 我原先每个调用各发一条消息，golden/S06（并行两个工具调用）的第二个请求
                    // 是 5 条 messages、我是 6 条，比对直接指出。
                    let calls: Vec<serde_json::Value> = assistant
                        .content
                        .iter()
                        .filter_map(|block| match block {
                            crate::Content::ToolCall {
                                id,
                                name,
                                arguments,
                                ..
                            } => Some(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(arguments)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                },
                            })),
                            crate::Content::Text { .. }
                            | crate::Content::Thinking { .. }
                            | crate::Content::Image { .. } => None,
                        })
                        .collect();
                    if !calls.is_empty() {
                        // 同一消息里的文本要与 tool_calls 一起发，且 `content` 是**字符串**
                        // （golden/S13b-toolcall 实测：`{"content":"开头","tool_calls":[…]}`）；
                        // 没有文本时是 `null`（golden/S02 实测：该字段存在且为 null）。
                        let text: String = assistant
                            .content
                            .iter()
                            .filter_map(|block| match block {
                                crate::Content::Text { text, .. } => Some(text.as_str()),
                                crate::Content::Thinking { .. }
                                | crate::Content::ToolCall { .. }
                                | crate::Content::Image { .. } => None,
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        let content = if text.is_empty() {
                            serde_json::Value::Null
                        } else {
                            serde_json::Value::String(text)
                        };
                        let mut message = json!({
                            "role": "assistant",
                            "content": content,
                            "tool_calls": calls,
                        });
                        Self::attach_reasoning(&mut message, provider, assistant);
                        out.push(message);
                    }
                    // **有 tool_calls 时不再单独合并文本**：文本已经进了那条消息的 `content`
                    // （字符串形态）。再走 `push_merged` 会把它按块数组并回同一条消息，
                    // 把刚写好的字符串覆盖掉——我第一版就是这样，比对里看到的仍是块数组。
                    if calls.is_empty() {
                        // **assistant 的 `content` 一律是纯字符串**（不是块数组）：
                        // ref `openai-completions.ts:1344-1349` 的注释原文是
                        // "Always send assistant content as a plain string (OpenAI Chat Completions
                        // API standard format)"——发块数组会被某些模型照着复读成嵌套结构。
                        // 文本块**空白的要滤掉**（`:1290-1297` 先 filter(trim > 0) 再 join("")）。
                        // 我此前用 `push_merged` 发块数组：回放历史时被 S11 的 C 层第 3 个请求指出。
                        let text: String = assistant
                            .content
                            .iter()
                            .filter_map(|block| match block {
                                crate::Content::Text { text, .. } if !text.trim().is_empty() => {
                                    Some(text.as_str())
                                }
                                crate::Content::Text { .. }
                                | crate::Content::Thinking { .. }
                                | crate::Content::ToolCall { .. }
                                | crate::Content::Image { .. } => None,
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        // ref `:1385-1388`：既无正文又无 tool_call 的 assistant **不进请求体**
                        // （"Skip assistant messages that have no content and no tool calls"）。
                        if !text.is_empty() {
                            let mut message = json!({ "role": "assistant", "content": text });
                            Self::attach_reasoning(&mut message, provider, assistant);
                            out.push(message);
                        }
                    }
                }
                // 三个 **agent 层**角色：正常情况下 `to_llm_messages` 已把它们折成 user
                // （ref `core/messages.ts:162-183` 的 `convertToLlm`）；这里仍给出等价渲染——
                // 穷尽 match 必须有分支，且**不得静默丢弃**。
                Message::Custom { content, .. } => {
                    out.push(json!({
                        "role": "user",
                        "content": user_blocks_for_openai(content),
                    }));
                }
                Message::BranchSummary { summary, .. } => {
                    out.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "text",
                            "text": format!(
                                "{}{}{}",
                                crate::message::BRANCH_SUMMARY_PREFIX,
                                summary,
                                crate::message::BRANCH_SUMMARY_SUFFIX
                            ),
                        }],
                    }));
                }
                Message::CompactionSummary { summary, .. } => {
                    out.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "text",
                            "text": format!(
                                "{}{}{}",
                                crate::message::COMPACTION_SUMMARY_PREFIX,
                                summary,
                                crate::message::COMPACTION_SUMMARY_SUFFIX
                            ),
                        }],
                    }));
                }
                Message::User { content, .. } => {
                    use crate::message::UserContent;
                    // **一条输入消息只产出一条输出消息**：把该消息的全部内容块组装成一个
                    // `content` 数组，一次 push。跨消息合并是错的——golden/S27 实测：
                    // 相邻的三个 user 消息（原始问题 / 自定义消息 / 分支摘要 / 继续）在请求体里
                    // 是**四条独立消息**，我此前合并成了一条（`push_merged` 的旧行为）。
                    let blocks: Vec<serde_json::Value> = match content {
                        UserContent::Text(text) => {
                            vec![serde_json::json!({ "type": "text", "text": text })]
                        }
                        UserContent::Blocks(items) => items
                            .iter()
                            .flat_map(|block| {
                                content_to_openai(block)
                                    .as_array()
                                    .cloned()
                                    .unwrap_or_default()
                            })
                            .collect(),
                    };
                    out.push(serde_json::json!({ "role": "user", "content": blocks }));
                }
            }
        }
        out
    }
}

/// 会话内容块 → OpenAI 的 `content` 片段（数组形态，与旧 `block_to_openai` 一致）。
fn content_to_openai(block: &crate::Content) -> Value {
    match block {
        crate::Content::Text { text, .. } => json!([{ "type": "text", "text": text }]),
        crate::Content::Image {
            data, mime_type, ..
        } => json!([{
            "type": "image_url",
            "image_url": { "url": format!("data:{mime_type};base64,{data}") },
        }]),
        crate::Content::Thinking { .. } | crate::Content::ToolCall { .. } => json!([]),
    }
}

/// 该消息在请求体里是否是 `user` 角色。
///
/// **`ToolResult` 也算**：会话里它是独立消息，请求体里是 user 消息里的
/// `tool_result` 块（`edit-diff`/`transform-messages` 侧的既定形状）。
/// ref `context.rs` 的 User 分支等价物：`UserContent` → OpenAI 的 `content` 数组。
fn user_blocks_for_openai(content: &crate::message::UserContent) -> Vec<Value> {
    match content {
        crate::message::UserContent::Text(text) => vec![json!({ "type": "text", "text": text })],
        crate::message::UserContent::Blocks(items) => items
            .iter()
            .flat_map(|block| {
                content_to_openai(block)
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .collect(),
    }
}

fn is_user_role(message: &crate::message::Message) -> bool {
    matches!(
        message,
        crate::message::Message::User { .. } | crate::message::Message::ToolResult { .. }
    )
}

/// `Message` → `(role, 块数组)`；`None` 表示该消息不进请求体。
fn message_to_anthropic(message: &crate::message::Message) -> Option<(&'static str, Vec<Value>)> {
    use crate::message::{Message, ToolResultContent};
    match message {
        // system **不进 messages**（与旧实现、与 `Context::system` 的分工一致）：
        // 它在 `from_messages` 里被搬到 `Context.system`，请求体里由 `system` 字段承载。
        Message::System { .. } => None,
        // 三个 agent 层角色（同上：`to_llm_messages` 之后通常不会走到这里，但必须有分支且不得静默丢弃）。
        Message::Custom { content, .. } => {
            Some(("user", user_content_to_anthropic(content, false)))
        }
        Message::BranchSummary { summary, .. } => Some((
            "user",
            vec![json!({
                "type": "text",
                "text": format!(
                    "{}{}{}",
                    crate::message::BRANCH_SUMMARY_PREFIX,
                    summary,
                    crate::message::BRANCH_SUMMARY_SUFFIX
                ),
            })],
        )),
        Message::CompactionSummary { summary, .. } => Some((
            "user",
            vec![json!({
                "type": "text",
                "text": format!(
                    "{}{}{}",
                    crate::message::COMPACTION_SUMMARY_PREFIX,
                    summary,
                    crate::message::COMPACTION_SUMMARY_SUFFIX
                ),
            })],
        )),
        Message::User { content, .. } => Some(("user", user_content_to_anthropic(content, false))),
        Message::Assistant(assistant) => Some((
            "assistant",
            assistant
                .content
                .iter()
                .map(|block| content_to_anthropic(block, false))
                .collect(),
        )),
        Message::ToolResult {
            tool_call_id,
            content,
            is_error,
            ..
        } => {
            // oracle 实测：单个文本块时 TS 发**字符串**，多块时才发块数组。
            let payload = match content.as_slice() {
                [ToolResultContent::Text { text, .. }] => Value::String(text.clone()),
                other => Value::Array(other.iter().map(tool_result_content_to_anthropic).collect()),
            };
            Some((
                "user",
                vec![json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": payload,
                    "is_error": is_error,
                })],
            ))
        }
    }
}

/// `UserContent`（字符串或块数组）→ Anthropic 块数组。
fn user_content_to_anthropic(
    content: &crate::message::UserContent,
    inside_tool_result: bool,
) -> Vec<Value> {
    use crate::message::UserContent;
    match content {
        UserContent::Text(text) => vec![json!({ "type": "text", "text": text })],
        UserContent::Blocks(blocks) => blocks
            .iter()
            .map(|block| content_to_anthropic(block, inside_tool_result))
            .collect(),
    }
}

/// 会话内容块 → Anthropic 块。
///
/// **图片块在两个位置形状不同**（golden/S25-anthropic 实测）：
/// - 普通 user 图片块：`source` **不带** `media_type`；
/// - `tool_result` 里的图片块：`source` **带** `media_type`。
fn content_to_anthropic(block: &crate::Content, inside_tool_result: bool) -> Value {
    use crate::Content;
    match block {
        Content::Text { text, .. } => json!({ "type": "text", "text": text }),
        Content::Thinking {
            thinking,
            thinking_signature,
            ..
        } => {
            // ref `anthropic-messages.ts:1330-1352`：
            // - `:1330` `hasThinkingSignature = !!sig && sig.trim().length > 0`
            // - `:1331` 空 thinking 且无签名 → **整块跳过**
            // - `:1335-1347` 无签名时按 `allowEmptySignature`（= `model.compat?.allowEmptySignature ?? false`，`:215`；
            //   默认 **false**）→ 降级为 `{type:"text", text}`
            // - `:1348-1352` 有签名时 → `{type:"thinking", thinking, signature}`
            let signature = thinking_signature.as_deref().unwrap_or_default();
            let has_signature = !signature.trim().is_empty();
            if thinking.trim().is_empty() && !has_signature {
                return Value::Null; // 调用方过滤掉（见下方 skip_null_blocks）
            }
            if !has_signature {
                json!({ "type": "text", "text": thinking })
            } else {
                json!({ "type": "thinking", "thinking": thinking, "signature": signature })
            }
        }
        Content::Image {
            data, mime_type, ..
        } => {
            let mut source = json!({ "type": "base64" });
            if inside_tool_result {
                source["media_type"] = json!(mime_type);
            }
            source["data"] = json!(data);
            json!({ "type": "image", "source": source })
        }
        Content::ToolCall {
            id,
            name,
            arguments,
            ..
        } => json!({ "type": "tool_use", "id": id, "name": name, "input": arguments }),
    }
}

/// `tool_result` 的内部块 → Anthropic 块（图片块带 `media_type`）。
fn tool_result_content_to_anthropic(block: &crate::message::ToolResultContent) -> Value {
    use crate::message::ToolResultContent;
    match block {
        ToolResultContent::Text { text, .. } => json!({ "type": "text", "text": text }),
        ToolResultContent::Image {
            data, mime_type, ..
        } => json!({
            "type": "image",
            "source": { "type": "base64", "media_type": mime_type, "data": data },
        }),
    }
}

impl Context {
    /// 从**会话消息**（`Message`）派生请求上下文——P3.0 要的"单一源头"入口。
    ///
    /// 映射规则（与 golden/S25-anthropic 的请求体逐项对照过）：
    /// - `System` 进 [`Context::system`]，不进 `messages`；
    /// - `User` 的字符串/块数组 → user 消息；
    /// - `Assistant` 的 `Content` 直接对应请求体的 text / thinking / tool_use 块；
    /// - `ToolResult` → **user 消息里的 `tool_result` 块**（请求体形状如此，
    ///   而会话里它是独立消息——这一层差异在转换处消掉）；
    /// - `stopReason` 为 `error`/`aborted` 的 assistant **整条丢弃**（由
    ///   [`crate::message::transform_messages`] 统一处理，这里只做搬运）。
    pub fn from_messages(system: Option<String>, messages: &[crate::message::Message]) -> Self {
        // **转换规则作用于 `Message`**（P3.0）：先丢 error/aborted 的 assistant、
        // 为未闭合的 tool call 合成结果。规则**只有这一份实现**
        // （`crate::message::transform_messages`），原先作用在 `ContextBlock` 上的
        // 那份副本已删除。
        let transformed = crate::message::transform_messages(messages);
        Self {
            system,
            messages: transformed,
            cache_control_last_user: true,
            cache_control_system: true,
            instruction_role: None,
            reasoning_effort: None,
        }
    }
}
