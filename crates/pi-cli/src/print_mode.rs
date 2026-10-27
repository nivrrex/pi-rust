//! print 模式的输出与退出码（`modes/print-mode.ts:143-161`、`main.ts:976-979`）。
//!
//! **纯函数在此**：`text_output` 与 `exit_code_for` 不碰 I/O，便于单测；
//! 真正跑循环的 `run_print` 只做编排。

use std::path::PathBuf;
use std::sync::Arc;

use pi_ai::message::Message;
use pi_ai::{Provider, StopReason};
use pi_core::agent::{AgentEvent, AgentLoop, EventSink, LoopConfig};

/// 输出子模式。〔main.ts:124-126〕
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintOutput {
    Text,
    Json,
}

/// text 模式的正文：只取 assistant 消息里 `type === "text"` 的块，**逐块加换行**。
/// 〔print-mode.ts:149-153〕
pub fn text_output(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        if let Message::Assistant(assistant) = message {
            for block in &assistant.content {
                if let pi_ai::Content::Text { text, .. } = block {
                    out.push_str(text);
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// 退出码：只有**最后一条消息**是 assistant 且 `stopReason` 为 `error`/`aborted` 时才置 1。
///
/// 返回 `(退出码, 要写到 stderr 的行)`。stderr 行是 `errorMessage`，缺省时是
/// `Request ${stopReason}`。〔print-mode.ts:143-158〕
pub fn exit_code_for(messages: &[Message]) -> (u8, Vec<String>) {
    let Some(Message::Assistant(assistant)) = messages.last() else {
        return (0, Vec::new());
    };
    match assistant.stop_reason {
        StopReason::Error => {
            let text = assistant
                .error_message
                .clone()
                .unwrap_or_else(|| String::from("Request error"));
            (1, vec![text])
        }
        StopReason::Aborted => {
            let text = assistant
                .error_message
                .clone()
                .unwrap_or_else(|| String::from("Request aborted"));
            (1, vec![text])
        }
        // 正常结束、工具调用中、长度截断都不置错误码；`Pending`/`Deferred` 是流中途的
        // 占位状态（`types.ts` 的 `StopReason` 成员），按"没有错误"处理。
        StopReason::Stop
        | StopReason::ToolUse
        | StopReason::Length
        | StopReason::Pending
        | StopReason::Deferred => (0, Vec::new()),
    }
}

/// 按命令行开关准备工具集。〔args.ts:141-156 的开关语义〕
pub fn default_tools(parsed: &crate::args::Args) -> Vec<Arc<dyn pi_core::tools::tool::Tool>> {
    if parsed.no_tools {
        return Vec::new();
    }
    // **顺序照 TS 的默认 selectedTools**：`["read","bash","edit","write"]`
    // （`system-prompt.ts:48`）。这个顺序同时决定系统提示的 `<tools>` 段与请求体的 tools 数组
    // ——我原先按 `[read,write,edit,bash]` 排列，被 C 层比对抓出（golden 是 read,bash,edit,write）。
    let all: Vec<Arc<dyn pi_core::tools::tool::Tool>> = vec![
        Arc::new(pi_core::tools::read::ReadTool),
        Arc::new(pi_core::tools::bash::BashTool),
        Arc::new(pi_core::tools::edit::EditTool),
        Arc::new(pi_core::tools::write::WriteTool),
    ];
    // `powershell` **不在默认激活集**（上游 `sdk.ts:258` / `agent-session.ts:3283` 都是
    // `["read","bash","edit","write"]`），但要让 `--tools powershell` 能选中它。
    let optional: Vec<Arc<dyn pi_core::tools::tool::Tool>> = vec![
        Arc::new(pi_core::tools::powershell::PowerShellTool),
        // 只读工具（上游默认也不启用，需要 `--tools` 选中）
        Arc::new(pi_core::tools::ls::LsTool),
        Arc::new(pi_core::tools::grep::GrepTool),
        Arc::new(pi_core::tools::find::FindTool),
    ];
    if !parsed.tools.is_empty() {
        return all
            .into_iter()
            .chain(optional)
            .filter(|tool| parsed.tools.iter().any(|name| name == tool.name()))
            .collect();
    }
    if parsed.exclude_tools.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|tool| !parsed.exclude_tools.iter().any(|name| name == tool.name()))
        .collect()
}

/// 跑一次 print 模式。
pub struct PrintRun {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub api: String,
    /// provider **名**（与上面那个 `provider: Arc<dyn Provider>` 不是一回事）。
    pub provider_name: String,
    pub system: Option<String>,
    /// 系统消息的结构化段（进 `message_start` 的 `message.sections`）。
    pub system_sections: std::collections::BTreeMap<String, String>,
    /// `--continue`/`--resume` 时从已有会话重建的消息（非继续时为空）。
    pub initial_messages: Vec<Message>,
    /// OpenAI 侧 system 消息角色（推理模型 `developer`）。
    pub instruction_role: Option<String>,
    /// OpenAI 侧 `reasoning_effort`（思考等级）。
    pub reasoning_effort: Option<String>,
    /// 本轮是否新写了 system 条目（`--continue` 且会话里原先没有 system）。
    pub announce_system: bool,

    pub prompt: String,
    pub max_completion_tokens: u64,
    pub cwd: PathBuf,
    pub tools: Vec<Arc<dyn pi_core::tools::tool::Tool>>,
    pub output: PrintOutput,
    /// 事件出口（json 模式实时写 stdout；text 模式传 `NoopEventSink`）。
    pub sink: Arc<dyn EventSink>,
    /// 会话写入器（重试包装器要同时拿到它与事件出口）。
    pub writer: Option<crate::session_writer::SessionWriter>,
    /// 已写进会话文件的消息条数（含 `main` 建写入器时写的 system 那条）。
    pub written: usize,
    /// P5-3c：自动压缩的参数（`--no-session` 时为 `None`，没有会话文件就没有可压缩的上下文）。
    pub compaction: Option<crate::auto_compaction::CompactionConfig>,
}

impl PrintRun {
    /// 返回 `(stdout 文本, stderr 行, 退出码, 本次运行的消息)`。
    ///
    /// 把消息一并返回，是为了让调用方决定要不要落会话文件（`--no-session` 时不落）。
    pub async fn execute(mut self) -> (String, Vec<String>, u8, Vec<Message>) {
        let cancel = tokio_util::sync::CancellationToken::new();
        let retry_max = LoopConfig::default().max_retries;
        let retry_delay_ms = LoopConfig::default().retry_delay_ms;
        let mut agent_loop = AgentLoop::new(
            // 循环拿一份 Arc（压缩阶段还要用同一个 provider 发摘要请求）。
            std::sync::Arc::clone(&self.provider),
            self.tools,
            match self.output {
                PrintOutput::Text => Arc::new(NoopEventSink) as Arc<dyn EventSink>,
                PrintOutput::Json => self.sink.clone(),
            },
            LoopConfig {
                model: self.model,
                api: self.api,
                provider: self.provider_name,
                system: self.system,
                system_sections: self.system_sections,
                max_completion_tokens: self.max_completion_tokens,
                // `--continue` 时开头那几条（`initial_messages`）**不是新消息**，
                // 不该再发它们的 `message_start`/`message_end`——golden/S14 第二阶段直接发
                // 新 user 消息的事件；我此前恒为 0，于是先发了一条 system 的开始事件。
                announce_from: self.initial_messages.len(),
                announce_system: self.announce_system,
                instruction_role: self.instruction_role.clone(),
                reasoning_effort: self.reasoning_effort.clone(),
                // `toolsRemoved` 的差分依据：会话里最后一份已声明的工具（含 compaction 的
                // `systemMessage`，见 `last_declared_tools`）。读不到会话时为空 = 没有上一份。
                system_tools_previous: self
                    .writer
                    .as_ref()
                    .and_then(|writer| pi_core::session_io::SessionFile::read(writer.path()).ok())
                    .map(|file| pi_core::agent::last_declared_tools(&file.entries))
                    .unwrap_or_default(),
                ..LoopConfig::default()
            },
        );
        agent_loop.cwd = self.cwd;

        // —— 重试包装器（产品层）——
        // golden/S07：失败那轮**正常收尾**（`turn_end`+`agent_end`）→ `auto_retry_start`
        // → `entry_appended` → 新的一次 `agent_start` → … → `auto_retry_end` → `agent_settled`。
        // 第二轮输入是**累积 transcript 去掉失败 assistant**之后的序列
        // （`context_edit` 的 `replacement: null` 就是这个语义）。
        // `--continue` 时这里先放**已有会话的消息**（按文件顺序），再追加本轮的用户消息。
        // 非继续时 `initial_messages` 为空，行为与以前一致。
        let mut transcript = self.initial_messages.clone();
        transcript.push(Message::User {
            content: pi_ai::UserContent::Text(self.prompt.clone()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        });
        // **外层：溢出恢复的重试回合**（ref `agent-session.ts:2693-2696` 的 `willRetry`；
        // 压缩后要**重跑该回合**，golden/S13e 的事件序列里 `compaction_end` 之后有第二个
        // `agent_start`）。`overflow_recovered` 就是 ref 的 `_overflowRecoveryAttempted`。
        let mut overflow_recovered = false;
        // 外层循环每轮都会重新产生 `messages`；收尾代码（退出码/正文/返回）用它。
        // 循环体至少执行一次、且**跳出前**一定赋值 ⇒ 这里只声明不初始化（否则 clippy 报 never-read）。
        let final_messages: Vec<Message>;
        loop {
            let mut retry_overflow = false;
            let mut attempts: u32 = 0;
            let mut messages: Vec<Message>;
            loop {
                // 第二轮起：transcript 里的消息都已存在，不再重发它们的事件
                // （golden/S07 第二次 run 只有 assistant 的 `message_start`）。
                // 但 system 那条的事件是**每次 run 都会发**的（golden 第二轮没有它）——
                // 见下方对 system 分支的处理。
                // 第 0 轮：`--continue` 时开头那几条已在会话里、不是新消息（`initial_messages`）；
                // 重试轮：这一轮传进去的 `transcript` 都不是新消息。
                agent_loop.set_announce_from(if attempts == 0 {
                    self.initial_messages.len()
                } else {
                    transcript.len()
                });
                // 第二轮起告诉循环"这是第几次重试"：它在成功那轮的 `message_end` 后发 `auto_retry_end`。
                agent_loop.set_retry_attempt(if attempts == 0 { None } else { Some(attempts) });
                messages = match agent_loop
                    .run(
                        transcript.clone(),
                        &|_| Vec::new(),
                        &|_| Vec::new(),
                        &cancel,
                    )
                    .await
                {
                    Ok(round) => round,
                    Err(error) => return (String::new(), vec![error.to_string()], 1, Vec::new()),
                };
                // **跳过数与 `announce_from` 同源**：第 n 轮传进来的那几条（`transcript`）
                // 已在上一轮写过了，只有其后的新产出才是要写的。
                // 我原先用 `self.written`（累计条数），重试轮 `transcript` 被 `pop` 缩短后
                // 累计值对不上，导致**成功的 assistant 没写进会话**（golden/S07 第 8 行）。
                // `--continue` 时开头那几条已在文件里（`initial_messages`），不能再写一遍。
                let to_write_from = if attempts == 0 {
                    self.initial_messages.len()
                } else {
                    transcript.len()
                };
                if let Some(writer) = self.writer.as_mut() {
                    for message in messages.iter().skip(to_write_from) {
                        if let Err(error) = writer.append_message(message) {
                            eprintln!("pi-rs: 写会话失败：{error}");
                            break;
                        }
                    }
                }

                let has_error_assistant = matches!(messages.last(), Some(Message::Assistant(_)));
                let error_message = match messages.last() {
                    Some(Message::Assistant(assistant)) => assistant.error_message.clone(),
                    Some(Message::User { .. })
                    | Some(Message::System { .. })
                    | Some(Message::ToolResult { .. })
                    | Some(Message::Custom { .. })
                    | Some(Message::BranchSummary { .. })
                    | Some(Message::CompactionSummary { .. })
                    | None => None,
                };
                let retryable = messages.last().is_some_and(|message| match message {
                    Message::Assistant(assistant) => {
                        pi_ai::provider::ProviderError::is_retryable_message_text(
                            &assistant.stop_reason,
                            assistant.error_message.as_deref(),
                        )
                    }
                    Message::User { .. }
                    | Message::System { .. }
                    | Message::ToolResult { .. }
                    | Message::Custom { .. }
                    | Message::BranchSummary { .. }
                    | Message::CompactionSummary { .. } => false,
                });
                if !retryable || attempts >= retry_max {
                    // **成功时不发** `auto_retry_end`——它已由循环在该轮的 `message_end` 之后发出
                    // （golden/S07 的位置）。这里只在"最后仍是失败"时补发。
                    if attempts > 0 && (has_error_assistant && error_message.is_some()) {
                        self.sink.emit(AgentEvent::AutoRetryEnd {
                            success: false,
                            attempt: attempts,
                            final_error: error_message,
                        });
                    }
                    break;
                }

                attempts += 1;
                self.sink.emit(AgentEvent::AutoRetryStart {
                    attempt: attempts,
                    max_attempts: retry_max,
                    delay_ms: retry_delay_ms,
                    error_message: error_message.unwrap_or_else(|| String::from("Unknown error")),
                });
                transcript = messages.clone();
                transcript.pop();
                if let Some(writer) = self.writer.as_mut() {
                    if let Some(target) = writer.last_entry_id().map(str::to_string) {
                        match writer.append_context_edit(&target) {
                            Ok(entry) => self.sink.emit(AgentEvent::EntryAppended { entry }),
                            Err(error) => eprintln!("pi-rs: 写 context_edit 失败：{error}"),
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(retry_delay_ms)).await;
            }
            // P5-3c：**回合结束后的自动压缩**（ref `agent-session.ts:2579-2598`："Dispatch automatic
            // compaction after `agent_end`"；golden/S19 的事件顺序即 `agent_end` → `compaction_start`
            // → `compaction_end` → `agent_settled`）。
            let compaction_config = self.compaction.clone();
            let compaction_provider = std::sync::Arc::clone(&self.provider);
            let compaction_sink = std::sync::Arc::clone(&self.sink);
            if let Some(config) = compaction_config {
                if let Some(writer) = self.writer.as_mut() {
                    let assistant = messages.iter().rev().find_map(|message| {
                        if let Message::Assistant(assistant) = message {
                            Some((**assistant).clone())
                        } else {
                            None
                        }
                    });
                    if let Some(assistant) = assistant {
                        if let Ok(file) = pi_core::session_io::SessionFile::read(writer.path()) {
                            // **可变**：`_omitRecoveryAttempt` 会补一条 `context_edit`，必须并进这份 entries——
                            // 否则下面用的是**省略前的快照**，`prepare_compaction` 因"两个集合都空"
                            // 返回 None（S13e 实测：errorMessage="nothing to compact"、请求数 1）。
                            let mut entries = file.entries.clone();
                            if let Some(plan) = crate::auto_compaction::auto_compaction_plan(
                                &entries, &config, &assistant, true,
                            ) {
                                // `compaction_start` 必须在**摘要调用之前**发出（ref `:2769` 早于 `:2828`
                                // 的 `compact()`），所以先发事件再跑压缩。
                                if let Some(error_message) = plan.failure {
                                    // 只发失败事件、不压缩（ref `agent-session.ts:2671-2690`）。
                                    compaction_sink.emit(AgentEvent::CompactionEnd {
                                        reason: plan.reason.to_string(),
                                        result: None,
                                        aborted: false,
                                        will_retry: false,
                                        error_message: Some(error_message.to_string()),
                                    });
                                } else {
                                    // **`_omitRecoveryAttempt`**（ref `agent-session.ts:1015-1031`，调用点 `:2695`）：
                                    // Case 1（`will_retry`）先对**失败的助手消息**写一条 `appendContextEdit(targetId, null)`
                                    // 并 emit `entry_appended`——投影因此省略它，压缩才有切点（golden/S13e 的重试上下文即压缩摘要）。
                                    // ref `agent-session.ts:2655-2700`：回应以 context overflow或**可恢复的 length** 收尾时先写 `context_edit(replacement:null)` 并 emit `entry_appended`，与 `will_retry` 无关。
                                    {
                                        let target = entries.iter().rev().find_map(|entry| {
                                            if let pi_core::session::SessionEntry::Message {
                                                id,
                                                message: Message::Assistant(assistant),
                                                ..
                                            } = entry
                                            {
                                                matches!(
                                                    assistant.stop_reason,
                                                    pi_ai::types::StopReason::Error
                                                        | pi_ai::types::StopReason::Length
                                                )
                                                .then(|| id.clone())
                                            } else {
                                                None
                                            }
                                        });
                                        if let Some(target) = target {
                                            match writer.append_context_edit(&target) {
                                                Ok(entry) => {
                                                    // 把刚写的 `context_edit` 并进 entries（否则压缩用的是省略前的快照）。
                                                    if let Ok(parsed) = serde_json::from_value::<
                                                        pi_core::session::SessionEntry,
                                                    >(
                                                        entry.clone()
                                                    ) {
                                                        entries.push(parsed);
                                                    }
                                                    compaction_sink
                                                        .emit(AgentEvent::EntryAppended { entry });
                                                }
                                                Err(error) => {
                                                    eprintln!(
                                                        "pi-rs: 写 context_edit 失败：{error}"
                                                    )
                                                }
                                            }
                                        }
                                    }
                                    // **不预先发 `CompactionStart`**：`perform_auto_compaction`
                                    // 返回 `Ok(None)`（"nothing to compact"）时 TS **一个事件都不发**
                                    // （X3 实测：`openai-responses` 的 `incomplete` 回合里我们多出
                                    // 一对 compaction_start/end）。改为真正要压缩时才发。
                                    match crate::auto_compaction::perform_auto_compaction(
                                        &entries,
                                        &config,
                                        None,
                                        Some(plan.reason),
                                        Some(compaction_sink.as_ref()),
                                        compaction_provider.as_ref(),
                                        writer,
                                    )
                                    .await
                                    {
                                        Ok(Some(run)) => {
                                            compaction_sink.emit(AgentEvent::CompactionStart {
                                                reason: plan.reason.to_string(),
                                            });
                                            if plan.will_retry {
                                                // 换用重建后的上下文，稍后重跑该回合。
                                                retry_overflow = true;
                                                // **三处都要换**：`initial_messages`（写入/announce 的基准）、`transcript`（下一轮
                                                // 循环的**输入**）与 `written`（已写条数）。只改前者时，重试轮仍拿旧上下文跑、
                                                // 且新产物被当成"已写过"跳过（S13e 的 A/B 两处差异即此）。
                                                transcript = run.messages.clone();
                                                self.written = 1 + run.messages.len();
                                                self.initial_messages = run.messages.clone();
                                            }
                                            compaction_sink.emit(AgentEvent::CompactionEnd {
                                                reason: plan.reason.to_string(),
                                                result: Some(run.result_event),
                                                aborted: false,
                                                will_retry: plan.will_retry,
                                                error_message: None,
                                            });
                                        }
                                        Ok(None) => {
                                            // 没有可压缩内容：ref 的 `_runAutoCompaction`
                                            // **不产生任何 compaction 事件**（X03 实测 TS 只有
                                            // 那条 `entry_appended(context_edit)`）。保持静默。
                                        }
                                        Err(error) => {
                                            compaction_sink.emit(AgentEvent::CompactionStart {
                                                reason: plan.reason.to_string(),
                                            });
                                            compaction_sink.emit(AgentEvent::CompactionEnd {
                                                reason: plan.reason.to_string(),
                                                result: None,
                                                aborted: false,
                                                will_retry: false,
                                                error_message: Some(error),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if retry_overflow && !overflow_recovered {
                overflow_recovered = true;
                continue;
            }
            // 只有确定要跳出时才记（否则这一轮的赋值会被下一轮覆盖 ⇒ clippy 的 never-read）。
            final_messages = messages.clone();
            break;
        }
        self.sink.emit(AgentEvent::AgentSettled);

        let (code, stderr) = exit_code_for(&final_messages);
        match self.output {
            PrintOutput::Text => (text_output(&final_messages), stderr, code, final_messages),
            // json 模式的正文由 sink 在循环中逐行写出；此处不再返回正文。
            PrintOutput::Json => (String::new(), stderr, code, final_messages),
        }
    }
}

/// json 投影失败的原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JsonEventError {
    /// `message_update` 的消息不是 assistant。〔json-event.ts:52-54〕
    #[error("message_update message is not an assistant message")]
    NotAnAssistantMessage,
    /// `toolcall_start` 指向的内容块不是工具调用。〔json-event.ts:23-30〕
    #[error("toolcall_start content at index {index} is not a tool call")]
    NotAToolCall { index: usize },
}

/// 把循环事件投影成 json 模式的线格式。〔`spec/b-agent-loop.md` 第 3 节〕
///
/// - `message_update` 重写成 `{ type, usage, assistantMessageEvent }`，`usage` 取**累积**
///   用量（`event.message.usage`）；
/// - **剥掉 `partial`**（`WithoutPartial<T>`）——累积快照不进线上协议；
/// - `toolcall_start` 额外附上从 `partial.content[contentIndex]` 取出的 `id` 与 `toolName`，
///   取不到就报错；
/// - 非 assistant 的 `message_update` 直接报错。
/// - 其余事件保留 `type` 与已有字段（本项目尚未逐字段对齐线上形状，见 notes）。
///
/// 事件行文本：与 [`to_json_event`] 相同，但对**带原始键的 system 消息**用原文拼。
///
/// 为什么需要：`Message::System` 的手写 `Serialize` 会用 `extra` 里的
/// `__toolsAddedRaw`/`__sectionsRaw` **原样写出**（会话文件那一行因此是对的），
/// 但 `serde_json::to_value(message)` 会把内部原始键"消费"成普通 `Value` ⇒ 键序被排序。
/// 而 `--continue` 载入**没有 system 条目**的会话时会新写并 announce 一条 system，
/// 事件载荷的键序差异就此显形（golden/S20c 的 B 层首差）。
pub fn to_json_event_line(event: &AgentEvent) -> String {
    let value = match to_json_event(event) {
        Ok(value) => value,
        Err(error) => serde_json::json!({
            "type": event.kind(),
            "projectionError": error.to_string(),
        }),
    };
    let mut line = value.to_string();
    if let AgentEvent::MessageStart { message } | AgentEvent::MessageEnd { message } = event {
        if matches!(message, Message::System { .. }) {
            // 手写 `Serialize` ⇒ 原文（内部键在这里被消费成 sections/toolsAdded 的原文）
            if let Ok(raw) = serde_json::to_string(message) {
                if let Some(replaced) =
                    pi_core::session_io::replace_json_field_value(&line, "message", &raw)
                {
                    line = replaced;
                }
            }
        }
    }
    line
}

pub fn to_json_event(event: &AgentEvent) -> Result<serde_json::Value, JsonEventError> {
    use serde_json::json;
    match event {
        AgentEvent::MessageUpdate {
            message,
            assistant_event,
        } => {
            let Message::Assistant(assistant) = message else {
                return Err(JsonEventError::NotAnAssistantMessage);
            };
            let event_value = match assistant_event {
                Some(pi_ai::AssistantEvent::ToolCallStart { content_index }) => {
                    // 从累积快照里取出该下标的内容块；不是工具调用就报错。
                    match assistant.content.get(*content_index) {
                        Some(pi_ai::Content::ToolCall { id, name, .. }) => json!({
                            "type": "toolcall_start",
                            "contentIndex": content_index,
                            "id": id,
                            "toolName": name,
                        }),
                        Some(_) | None => {
                            return Err(JsonEventError::NotAToolCall {
                                index: *content_index,
                            })
                        }
                    }
                }
                Some(pi_ai::AssistantEvent::Start) => json!({ "type": "start" }),
                Some(pi_ai::AssistantEvent::TextStart { content_index }) => {
                    json!({ "type": "text_start", "contentIndex": content_index })
                }
                Some(pi_ai::AssistantEvent::TextDelta {
                    content_index,
                    delta,
                }) => {
                    json!({ "type": "text_delta", "contentIndex": content_index, "delta": delta })
                }
                Some(pi_ai::AssistantEvent::TextEnd {
                    content_index,
                    content,
                }) => {
                    json!({ "type": "text_end", "contentIndex": content_index, "content": content })
                }
                Some(pi_ai::AssistantEvent::ThinkingStart { content_index }) => {
                    json!({ "type": "thinking_start", "contentIndex": content_index })
                }
                Some(pi_ai::AssistantEvent::ThinkingDelta {
                    content_index,
                    delta,
                }) => {
                    json!({ "type": "thinking_delta", "contentIndex": content_index, "delta": delta })
                }
                Some(pi_ai::AssistantEvent::ThinkingEnd {
                    content_index,
                    content,
                }) => {
                    json!({ "type": "thinking_end", "contentIndex": content_index, "content": content })
                }
                Some(pi_ai::AssistantEvent::ToolCallDelta {
                    content_index,
                    delta,
                }) => {
                    json!({ "type": "toolcall_delta", "contentIndex": content_index, "delta": delta })
                }
                Some(pi_ai::AssistantEvent::ToolCallEnd { content_index }) => {
                    json!({ "type": "toolcall_end", "contentIndex": content_index })
                }
                // `done` / `error` 都带上当前累积消息：TS 分别是 `message` 与 `error`
                // 字段（`ai/src/types.ts:663-668`）。
                Some(pi_ai::AssistantEvent::Done { reason }) => json!({
                    "type": "done",
                    "reason": reason,
                    "message": serde_json::to_value(assistant).unwrap_or(serde_json::Value::Null),
                }),
                Some(pi_ai::AssistantEvent::Error { reason, .. }) => json!({
                    "type": "error",
                    "reason": reason,
                    "error": serde_json::to_value(assistant).unwrap_or(serde_json::Value::Null),
                }),
                None => json!({ "type": "start" }),
            };
            Ok(json!({
                "type": "message_update",
                "usage": serde_json::to_value(assistant.usage).unwrap_or(serde_json::Value::Null),
                "assistantMessageEvent": event_value,
            }))
        }
        AgentEvent::AutoRetryStart {
            attempt,
            max_attempts,
            delay_ms,
            error_message,
        } => Ok(json!({
            "type": "auto_retry_start",
            "attempt": attempt,
            "maxAttempts": max_attempts,
            "delayMs": delay_ms,
            "errorMessage": error_message,
        })),
        AgentEvent::AutoRetryEnd {
            success,
            attempt,
            final_error,
        } => {
            let mut value = json!({
                "type": "auto_retry_end",
                "success": success,
                "attempt": attempt,
            });
            if let Some(final_error) = final_error {
                value["finalError"] = json!(final_error);
            }
            Ok(value)
        }
        // 其余事件按线上形状带上载荷。golden/S01 的 B 层实测：
        // `message_start`/`message_end` 带 `message`，`turn_end` 带 `message` 与 `toolResults`，
        // `agent_end` 带 `messages`。我原先只输出 `type`，载荷全丢。
        AgentEvent::MessageStart { message } | AgentEvent::MessageEnd { message } => Ok(json!({
            "type": event.kind(),
            "message": serde_json::to_value(message).unwrap_or(serde_json::Value::Null),
        })),
        // print 模式没有 steer/follow-up 队列，两个列表恒为空。
        AgentEvent::QueueUpdate => Ok(json!({
            "type": "queue_update",
            "steering": [],
            "followUp": [],
        })),
        AgentEvent::TurnEnd {
            message,
            tool_results,
        } => Ok(json!({
            "type": "turn_end",
            "message": serde_json::to_value(message).unwrap_or(serde_json::Value::Null),
            "toolResults": serde_json::to_value(tool_results).unwrap_or(serde_json::Value::Null),
        })),
        AgentEvent::AgentEnd {
            messages,
            will_retry,
        } => Ok(json!({
            "type": "agent_end",
            "messages": serde_json::to_value(messages).unwrap_or(serde_json::Value::Null),
            "willRetry": will_retry,
        })),
        AgentEvent::TurnStart => Ok(json!({ "type": "turn_start" })),
        AgentEvent::EntryAppended { entry } => {
            Ok(json!({ "type": "entry_appended", "entry": entry }))
        }
        AgentEvent::CompactionStart { reason } => {
            Ok(json!({ "type": "compaction_start", "reason": reason }))
        }
        // ref `agent-session.ts:2866`（成功）/`:2882-2888`（失败或中止）：`result` 只在成功时有，
        // `errorMessage` 只在失败时有（golden/S19 的成功路径是 `result` + `aborted:false`）。
        AgentEvent::CompactionEnd {
            reason,
            result,
            aborted,
            will_retry,
            error_message,
        } => {
            let mut value = json!({
                "type": "compaction_end",
                "reason": reason,
                "aborted": aborted,
                "willRetry": will_retry,
            });
            if let Some(object) = value.as_object_mut() {
                if let Some(result) = result {
                    object.insert("result".to_string(), result.clone());
                }
                if let Some(error_message) = error_message {
                    object.insert("errorMessage".to_string(), json!(error_message));
                }
            }
            Ok(value)
        }
        AgentEvent::SummarizationRetryScheduled {
            attempt,
            max_attempts,
            delay_ms,
            error_message,
        } => Ok(json!({
            "type": "summarization_retry_scheduled",
            "attempt": attempt,
            "maxAttempts": max_attempts,
            "delayMs": delay_ms,
            "errorMessage": error_message,
        })),
        AgentEvent::SummarizationRetryAttemptStart { source, reason } => {
            let mut value = json!({
                "type": "summarization_retry_attempt_start",
                "source": source,
            });
            if let Some(reason) = reason {
                if let Some(object) = value.as_object_mut() {
                    object.insert("reason".to_string(), json!(reason));
                }
            }
            Ok(value)
        }
        AgentEvent::SummarizationRetryFinished => {
            Ok(json!({ "type": "summarization_retry_finished" }))
        }
        AgentEvent::AgentSettled => Ok(json!({ "type": "agent_settled" })),
        AgentEvent::AgentStart => Ok(json!({ "type": "agent_start" })),
        AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => Ok(json!({
            "type": "tool_execution_start",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "args": args,
        })),
        AgentEvent::ToolExecutionUpdate {
            tool_call_id,
            tool_name,
            args,
            partial,
        } => Ok(json!({
            "type": "tool_execution_update",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "args": args,
            // 字段名是 `partialResult`（golden/S05 实测；我原先写成 `partial`）。
            "partialResult": partial,
        })),
        AgentEvent::ToolExecutionEnd {
            tool_call_id,
            tool_name,
            result,
            is_error,
        } => Ok(json!({
            "type": "tool_execution_end",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "result": result,
            "isError": is_error,
        })),
    }
}

/// 什么都不做的 sink（text 模式用）。
#[derive(Debug, Default)]
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _event: AgentEvent) {}
}

/// **逐事件实时**写到 stdout 的 sink（json 模式用）。
///
/// 为什么必须实时：runner 的 `waitEvent` 要**在运行中**等到事件才能继续驱动场景
/// （放行闸门、创建文件）。把事件攒到运行结束再输出，S09a/S09b/S10/S11 这类场景会直接卡死
/// ——这正是它们此前"无输出"的原因。TS 的 json 模式也是逐事件写。
pub struct StdoutJsonSink;

impl EventSink for StdoutJsonSink {
    fn emit(&self, event: AgentEvent) {
        use std::io::Write;
        let line = to_json_event_line(&event);
        let mut stdout = std::io::stdout();
        // 写失败（下游关闭管道）不 panic：只丢弃这一行。
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
}

/// 把事件写成 JSON 行（json 模式的线格式）。
///
/// `message_update` 的**投影**见 `spec/b-agent-loop.md` 第 3 节：`{type, usage,
/// assistantMessageEvent}`，且**剥掉 `partial`**。本实现目前只输出 `type` 与
/// 关键字段，事件类型与 `AgentEvent::kind()` 一致——完整投影未做，见 notes。
pub struct JsonLineSink {
    lines: std::sync::Mutex<Vec<String>>,
}

impl Default for JsonLineSink {
    fn default() -> Self {
        Self {
            lines: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl JsonLineSink {
    pub fn snapshot(&self) -> Vec<String> {
        self.lines
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

impl EventSink for JsonLineSink {
    fn emit(&self, event: AgentEvent) {
        // 投影失败**不静默吞掉**：写成一行带 `projectionError` 的记录，让问题可见
        // （TS 侧是直接抛错，见 `json-event.ts:23-30,52-54`）。
        let line = to_json_event_line(&event);
        if let Ok(mut guard) = self.lines.lock() {
            guard.push(line);
        }
    }
}
