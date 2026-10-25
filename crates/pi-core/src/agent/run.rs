//! agent loop 的 I/O 驱动：发请求、跑工具、把过程发成事件。
//!
//! 调度决策全部交给纯状态机（[`super`]），本模块只负责"真的去做"。
//!
//! **并发与所有权**（AGENTS 要求公共 API 声明线程安全性）：
//! - `AgentLoop` 持 `Arc<dyn Provider>`、`Arc<HashMap<String, Arc<dyn Tool>>>` 与 `Arc<dyn EventSink>`，
//!   三者都要求 `Send + Sync`；`AgentLoop` 自身无可变字段，`run()` 可并发调用；
//! - 一次 run 的运行态（上下文、本轮结果）是 `run()` 的局部变量，不共享；
//! - 取消经 [`CancellationToken`] 传播：请求与每个工具执行都用同一个 token 的引用，
//!   已取消时尚未开始的工具调用会被截断（产出 `Operation aborted`，见状态机模块）。

use std::sync::Arc;

use futures::future::join_all;
use pi_ai::message::Message;
use pi_ai::{AssistantEvent, Provider, ProviderRequest, StopReason};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::event::AgentEvent;
use super::{
    aborted_call_outcomes, after_follow_up, after_turn, batch_execution_mode, length_limit_error,
    should_terminate_batch, DrainMode, ExecutionMode, FinishDecision, TurnAction, TurnOutcome,
};
use crate::tools::tool::{Tool, ToolContext, ToolResult};

/// 事件出口。实现方必须是 `Send + Sync`（循环可能在并发路径里发事件）。
pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);

    /// 发出事件，并**开启一个原子段**：本线程的下一次 `emit` 之前，其他线程不得写入事件流。
    ///
    /// 用来复刻 ref 的单线程语义（见 `agent/section.rs`）：工具入口的首次进度上报发生在任何
    /// `await` 之前（`coding-agent/src/core/tools/bash.ts:301-302`），RPC 侧的命令处理只能在 await 点
    /// 被调度，所以 `tool_execution_start` 与工具的首条 `tool_execution_update` 之间不可能插进
    /// `queue_update`。默认实现（print 模式等单线程出口）就是普通 `emit`。
    fn emit_opening_section(&self, event: AgentEvent) {
        self.emit(event);
    }

    /// 结束本线程开启的原子段（没有开启时是空操作）。工具返回后兜底调用。
    fn end_section(&self) {}
}

/// 丢弃全部事件。
#[derive(Debug, Default)]
pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: AgentEvent) {}
}

/// 收集事件到内存，供测试断言。
#[derive(Debug, Default)]
pub struct CollectSink {
    events: std::sync::Mutex<Vec<AgentEvent>>,
}

impl CollectSink {
    /// 取当前事件快照（按发出顺序）。
    pub fn snapshot(&self) -> Vec<AgentEvent> {
        self.events
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

impl EventSink for CollectSink {
    fn emit(&self, event: AgentEvent) {
        if let Ok(mut guard) = self.events.lock() {
            guard.push(event);
        }
    }
}

/// 循环的静态配置。
#[derive(Debug, Clone)]
pub struct LoopConfig {
    pub model: String,
    /// provider 的 API 标识（如 `openai-completions`）与 provider 名。
    /// 进 assistant 消息的 `api`/`provider` 字段（golden/S01 的 `message_start` 实测）。
    pub api: String,
    pub provider: String,
    pub system: Option<String>,
    /// 系统消息的**结构化段**（`{段名: 文本}`）。进 `message_start` 的 `message.sections`。
    /// 出处：`SystemMessage.sections`（`agent-session.ts` 传给 `buildSystemPromptState`）。
    pub system_sections: std::collections::BTreeMap<String, String>,
    /// **上一份**工具声明（`toolsAdded` 的 JSON 数组）：用于 `toolsRemoved` 的差分
    /// （ref `packages/ai/src/utils/transcript.ts:150-167` 的 `previous`）。
    /// 由调用方从会话里最后一条 system 消息读出来；空表示"没有上一份"。
    pub system_tools_previous: Vec<serde_json::Value>,
    pub max_completion_tokens: u64,
    /// 工具批次的模式（配置层）。
    pub tool_execution: ExecutionMode,
    /// 队列取出模式。〔agent.ts:244-245〕
    pub steering_mode: DrainMode,
    pub follow_up_mode: DrainMode,
    /// 每轮结束时的 `finishTurn` 决定；测试与产品层注入。
    pub finish: FinishDecision,
    /// 从第几条消息开始为"**本轮新增**"的消息发 `message_start`/`message_end`。
    ///
    /// TS 只为**待处理**（尚未进上下文）的消息发这一对事件（`agent-loop.ts:209-216`）：
    /// 第二次 run 时 system/user 都已在 transcript 里，所以**不重发**——golden/S07 的
    /// 第二次 run 只有 assistant 的 `message_start`。默认 0 表示"全部都是新的"。
    pub announce_from: usize,
    /// 本轮是否**新写入了 system 条目**（`--continue` 且会话里原先没有 system 时为真）。
    /// 这种情况下 system 消息是**本轮的待处理消息**，要在 `turn_start` 后 announce 一次
    /// （golden/S17 的 B 层实测：助手之前有 `system` 与 `user` **两对** `message_start/end`）。
    pub announce_system: bool,
    /// OpenAI 侧 system 消息角色（推理模型 `developer`，`openai-completions.ts:1225`）。
    pub instruction_role: Option<String>,
    /// OpenAI 侧 `reasoning_effort`（思考等级，`openai-completions.ts:925-928`）。
    pub reasoning_effort: Option<String>,
    /// 本次 run 是"重试的第几次"。`Some(n)` 表示：本轮若**没有**以 error 结束，
    /// 就在该轮的 `message_end` 之后立刻发 `auto_retry_end{success:true, attempt:n}`
    /// ——golden/S07 的顺序是 `… message_end, auto_retry_end, turn_end, agent_end`。
    pub retry_attempt: Option<u32>,
    /// 产品层重试的最大次数（**首次调用不算一次重试**）。
    ///
    /// 默认 3，取自 `golden/S07` 的 `auto_retry_start` 事件里 `maxAttempts: 3`。
    pub max_retries: u32,
    /// 重试前的固定退避毫秒数。默认 2000（同 golden/S07 的 `delayMs`）。
    ///
    /// 注：TS 的退避是 `baseDelayMs * 2^n` 的指数形式（`retry.ts` 的 `RetryPolicy`），
    /// 我这里只实现了常量档——S07 只重试一次，指数与常量的结果相同，因此
    /// **多档退避尚未被证据覆盖**（见 notes）。
    pub retry_delay_ms: u64,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            api: String::new(),
            provider: String::new(),
            system: None,
            system_sections: std::collections::BTreeMap::new(),
            system_tools_previous: Vec::new(),
            max_completion_tokens: 16384,
            tool_execution: ExecutionMode::Parallel,
            steering_mode: DrainMode::OneAtATime,
            follow_up_mode: DrainMode::OneAtATime,
            finish: FinishDecision::Undefined,
            announce_from: 0,
            announce_system: false,
            instruction_role: None,
            reasoning_effort: None,
            retry_attempt: None,
            max_retries: 3,
            retry_delay_ms: 2000,
        }
    }
}

/// 循环失败。
#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error("provider 请求失败: {0}")]
    Provider(#[from] pi_ai::ProviderError),
    #[error("循环缺少可执行的 provider 或工具表")]
    MissingDependency,
}

/// 代理循环。
pub struct AgentLoop {
    provider: Arc<dyn Provider>,
    /// **有序列**（注册顺序 = 请求体里的工具顺序）。
    tools: Arc<Vec<Arc<dyn Tool>>>,
    sink: Arc<dyn EventSink>,
    config: LoopConfig,
    /// 工具执行时的工作目录。
    pub cwd: std::path::PathBuf,
}

impl AgentLoop {
    /// 设置"从第几条消息开始算作本轮新增"（重试包装器用）。
    /// 设置"这是第几次重试"（重试包装器用）。
    pub fn set_retry_attempt(&mut self, value: Option<u32>) {
        self.config.retry_attempt = value;
    }

    pub fn set_announce_from(&mut self, value: usize) {
        self.config.announce_from = value;
    }

    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Vec<Arc<dyn Tool>>,
        sink: Arc<dyn EventSink>,
        config: LoopConfig,
    ) -> Self {
        Self {
            provider,
            tools: Arc::new(tools),
            sink,
            config,
            cwd: std::path::PathBuf::from("."),
        }
    }

    /// 跑一次完整的 run：内层处理工具与 steering，外层处理 follow-up。
    ///
    /// `messages` 是本次 run 的初始上下文；`steering` / `follow_up` 是注入的队列取出函数
    /// （返回本次取到的消息，调用方按 `steering_mode`/`follow_up_mode` 决定取几条）。
    pub async fn run(
        &self,
        messages: Vec<Message>,
        steering: &(dyn Fn(DrainMode) -> Vec<Message> + Send + Sync),
        follow_up: &(dyn Fn(DrainMode) -> Vec<Message> + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<Vec<Message>, LoopError> {
        // 进入 transcript 的 user 消息**统一成块数组**：TS 侧 `content: string` 会在
        // 会话/事件里被规范成 `[{type:"text",text:…}]`（golden/S01 的 `message_start` 实测），
        // 我原先原样保留字符串，B 层比对直接指出。
        let mut transcript: Vec<Message> =
            messages.into_iter().map(normalize_user_content).collect();
        let mut new_messages: Vec<Message> = Vec::new();
        self.sink.emit(AgentEvent::AgentStart);

        let mut suggests_follow_up = false;
        let mut emitted_initial_messages = false;
        // steering 消费后要发 `queue_update`，且必须排在**下一回合的 `turn_start` 之后**
        // （golden/S10 的第 21 条紧跟第 20 条的 `turn_start`）：drain 发生在回合边界，
        // 而事件要等新回合开始才发，所以用一个标志延后。
        let mut queue_drained = false;
        // 本回合新追加的"待处理消息"（steering / follow-up）要各自发一对
        // `message_start`/`message_end`（`agent-loop.ts:209-216`，与初始消息同一规则）。
        // golden/S10 实测：`turn_start`(20) → `queue_update`(21) → **被 steer 的 user 消息
        // 的 message_start/end**(22/23) → 助手的流式 `message_start`(24)。
        let mut announce_pending: Vec<Message> = Vec::new();
        'outer: loop {
            // —— 内层：工具调用与 steering ——
            let mut pending: Vec<Message> = Vec::new();

            // 内层用带标签的块把"下一个动作 + explicit_continuation"作为**值**带出来，
            // 免得在循环外声明一个可变量（那会触发 clippy 的"赋值未读"）。
            let (inner_action, explicit_continuation) = 'inner: loop {
                self.sink.emit(AgentEvent::TurnStart);
                if queue_drained {
                    queue_drained = false;
                    self.sink.emit(AgentEvent::QueueUpdate);
                }
                for message in std::mem::take(&mut announce_pending) {
                    self.sink.emit(AgentEvent::MessageStart {
                        message: message.clone(),
                    });
                    self.sink.emit(AgentEvent::MessageEnd { message });
                }
                if !emitted_initial_messages {
                    emitted_initial_messages = true;
                    // 待处理消息在 turn_start **之后**各自发一对 `message_start`/`message_end`
                    // （`agent-loop.ts:209-216`）。golden/S01 的 B 层实测：第 4/5 行是 system、
                    // 第 6/7 行是 user，然后才是 assistant 的流式事件。
                    // system 事件只在首轮发（进了 transcript 后就不再是待处理消息）。
                    if self.config.announce_from == 0 || self.config.announce_system {
                        // **与文件侧同一份构造**：调 `system_message()`（它带 `sections`/`toolsAdded`
                        // 的原文键与 `toolsRemoved`）。此前这里**自己又拼了一遍**（只有 `sections` +
                        // `toolsAdded`、没有原文键、没有 `toolsRemoved`）⇒ 事件载荷与会话文件不一致
                        // （S20c 的 B 层就是这条：文件里有 `toolsRemoved`、事件里没有）。
                        if let Some(system_message) = self.system_message() {
                            self.sink.emit(AgentEvent::MessageStart {
                                message: system_message.clone(),
                            });
                            self.sink.emit(AgentEvent::MessageEnd {
                                message: system_message,
                            });
                        }
                    }
                    for message in transcript.iter().skip(self.config.announce_from) {
                        self.sink.emit(AgentEvent::MessageStart {
                            message: message.clone(),
                        });
                        self.sink.emit(AgentEvent::MessageEnd {
                            message: message.clone(),
                        });
                    }
                }
                // 推理模型才有这两项（golden/S16 实测；S01 等非推理场景不出现）。
                // ref `core/messages.ts:148-188`（`convertToLlm`）：agent 层的三个角色
                // （custom / branchSummary / compactionSummary）在**构造请求上下文之前**折成 user 消息，
                // provider 层因此只见到 4 个已知 role。既有场景里没有这三类消息 ⇒ 恒等变换、行为不变。
                let llm_transcript = pi_ai::message::to_llm_messages(&transcript);
                let mut context =
                    pi_ai::Context::from_messages(self.config.system.clone(), &llm_transcript);
                context.instruction_role = self.config.instruction_role.clone();
                context.reasoning_effort = self.config.reasoning_effort.clone();
                let request = ProviderRequest {
                    model: self.config.model.clone(),
                    context,
                    max_completion_tokens: self.config.max_completion_tokens,
                    tools: self.tool_specs(),
                    // anthropic 侧的 `thinking` 配置（P4-13 步骤 3）：按 ref
                    // `anthropic-messages.ts:1160-1182` + `simple-options.ts:79-95` 从**思考等级**算出。
                    // **只对 anthropic 生效**：openai 侧的请求构造不读 `request.thinking`
                    // （`crates/pi-ai/src/openai.rs` 里 grep `thinking` 零命中），
                    // 所以这里对 openai 保持 `None`，其请求体逐字节不变。
                    thinking: if self.config.api == "anthropic-messages" {
                        pi_ai::anthropic::thinking_config_for_level(
                            self.config.reasoning_effort.as_deref(),
                            None,
                            self.config.max_completion_tokens,
                        )
                    } else {
                        None
                    },
                    // 每次请求的**额外**头。ref 里 `model.headers` 属于**客户端默认头**
                    // （`openai-completions.ts:760`），由 `crates/pi-cli/src/runtime.rs` 的
                    // `build_provider` 交给 client；这里是 `providerOptions.headers` 那一层
                    // （无调用方就不发）→ 保持空是对的，不是"写死"。
                    headers: Vec::new(),
                };
                let final_message = self.stream_turn(request.clone(), cancel).await?;
                let stop_reason = final_message.stop_reason;
                let tool_calls = tool_calls_of(&final_message);

                let (tool_results, terminate) = if tool_calls.is_empty() {
                    (Vec::new(), false)
                } else {
                    self.run_batch(&final_message, &tool_calls, stop_reason, cancel)
                        .await
                };

                // 工具结果消息也要各发一对 `message_start`/`message_end`
                // （`agent-loop.ts:646-651` 的 `emitToolResultMessage`）。缺了它们，
                // golden/S02 的 B 层会多出两条事件（22 行 vs 我 20 行）。
                for message in &tool_results {
                    self.sink.emit(AgentEvent::MessageStart {
                        message: message.clone(),
                    });
                    self.sink.emit(AgentEvent::MessageEnd {
                        message: message.clone(),
                    });
                }
                self.sink.emit(AgentEvent::TurnEnd {
                    message: Message::Assistant(Box::new(final_message.clone())),
                    tool_results: tool_results.clone(),
                });

                let mut produced: Vec<Message> =
                    vec![Message::Assistant(Box::new(final_message.clone()))];
                produced.extend(tool_results.iter().cloned());
                for message in produced {
                    transcript.push(message.clone());
                    new_messages.push(message);
                }
                transcript.append(&mut pending);

                let has_more = !tool_calls.is_empty() && !terminate;
                // steering 只在**本轮工具执行结束后**拉取。〔agent-loop.ts:286-294〕
                let steering_messages = if has_more {
                    steering(self.config.steering_mode)
                } else {
                    Vec::new()
                };
                let outcome = TurnOutcome {
                    stop_reason,
                    has_more_tool_calls: has_more,
                    steering_count: steering_messages.len(),
                    follow_up_count: 0,
                    finish: self.config.finish,
                };
                let (action, explicit) = after_turn(&outcome);
                // 注：**不要**因"已取消"在这里强制 `EndRun`。ref `agent-loop.ts:310-313` 在
                // `finishTurn` 判 `continue` 时会再开一个"**纯上下文回合**"；那一轮因为信号已中止，
                // `stream_turn` 入口直接物化一条错误消息、**不发请求**（见该函数开头），
                // 于是 golden/S09b 的第 19-22 条（`turn_start` + ms/me + `turn_end`）才出现，
                // 且 `requests.jsonl` 仍只有 1 条。之后 `after_turn` 见 `stop_reason=Error` 自然收尾。
                match action {
                    TurnAction::ContinueInner => {
                        // 有 steering 被消费就置标志（空队列不置，避免多发事件）。
                        // steering 消息进 transcript 时**正文要规范化成块数组**：
                        // golden/S10 实测被 steer 的 user 消息 `content` 是
                        // `[{"text":"改用中文回答","type":"text"}]`，而我发的是字符串
                        // （初始 user 消息此前已规范化，这条路径漏了）。
                        let steering_messages: Vec<Message> = steering_messages
                            .into_iter()
                            .map(normalize_user_content)
                            .collect();
                        queue_drained = queue_drained || !steering_messages.is_empty();
                        announce_pending.extend(steering_messages.iter().cloned());
                        transcript.extend(steering_messages);
                        continue 'inner;
                    }
                    TurnAction::EndRun => break 'inner (TurnAction::EndRun, explicit),
                    TurnAction::PollFollowUp => break 'inner (TurnAction::PollFollowUp, explicit),
                }
            };
            match inner_action {
                TurnAction::EndRun => break 'outer,
                TurnAction::PollFollowUp => {}
                TurnAction::ContinueInner => unreachable!("内层不会以 ContinueInner 退出"),
            }

            // —— 外层：follow-up ——
            let follow_ups = follow_up(self.config.follow_up_mode);
            if !follow_ups.is_empty() {
                suggests_follow_up = false;
            }
            match after_follow_up(follow_ups.len(), explicit_continuation) {
                TurnAction::ContinueInner => {
                    let follow_ups: Vec<Message> =
                        follow_ups.into_iter().map(normalize_user_content).collect();
                    queue_drained = queue_drained || !follow_ups.is_empty();
                    announce_pending.extend(follow_ups.iter().cloned());
                    transcript.extend(follow_ups);
                    continue 'outer;
                }
                TurnAction::EndRun => break 'outer,
                TurnAction::PollFollowUp => unreachable!("after_follow_up 不返回 PollFollowUp"),
            }
        }
        let _ = suggests_follow_up;
        // `agent_end.messages` 是**完整 transcript**（golden/S01 实测首元素是 system 消息），
        // 不是本 run 新增的那几条。
        // `agent_end.messages` 是**完整 transcript**（golden/S01 实测首元素是 system 消息）。
        // system 提示在 `config` 里而非 `transcript` 里（请求体仍由 `Context.system` 承载），
        // 因此这里补到最前面。
        let retryable_failure = transcript.last().is_some_and(|message| match message {
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
        // `agent_end.messages` 是**本次 run 新增**的消息：
        // - 首轮（`announce_from == 0`）：system + user + 新产出的 assistant（golden/S01 三条）；
        // - 重试轮（`announce_from > 0`）：只有本轮新产出的那条 assistant
        //   （golden/S07 第二轮的 `agent_end.messages` 只有一条）。
        // `agent_end.messages` = **本次 run 新增**的消息：
        // - 传入的 `transcript` 里，`announce_from` 之后的那些是"新"的
        //   （首轮 = 初始 user；`--continue` 轮 = 本轮的新 user）；
        // - 再加上循环自己产出的（助手/工具结果）——它们也被 push 进了 `transcript`；
        // - 首轮另外把 system 放在最前（golden/S01 三条：system、user、assistant）。
        // golden/S14 第二阶段的 `agent_end.messages` 是 **[user「second turn」, assistant]**
        // ——我此前用 `new_messages`（只有助手），漏掉了传入的新 user。
        let mut full_transcript: Vec<Message> = transcript
            .iter()
            .skip(self.config.announce_from)
            .cloned()
            .collect();
        // 与"要不要 announce system"同一条件：`--continue` 且本轮补写了 system 时，
        // 它也是**本轮新增**的消息（golden/S17 的 `agent_end.messages` 首元素就是 system）。
        if self.config.announce_from == 0 || self.config.announce_system {
            if let Some(system_message) = self.system_message() {
                full_transcript.insert(0, system_message);
            }
        }
        self.sink.emit(AgentEvent::AgentEnd {
            messages: full_transcript,
            // `willRetry`：本轮以**可重试**的错误结束、且这还不是重试轮 → 后面会重试
            // （golden/S07 的失败那轮是 `true`）。我原先硬编码 `false`，被 B 层指出。
            will_retry: retryable_failure && self.config.retry_attempt.is_none(),
        });
        // `agent_settled` 由包装器在整次会话（含重试）结束时发。
        Ok(transcript)
    }

    /// 系统消息（`content` 为空串 + `sections` + `toolsAdded`）；未配置系统提示时为 `None`。
    ///
    /// 注：turn 内那处发射**仍各自构造一份**（未与此方法合并）——合并要动那段内联逻辑，
    /// 本轮上下文已耗尽，先留重复，记在 notes 里待清理。
    fn system_message(&self) -> Option<Message> {
        self.config.system.as_ref()?;
        let mut extra = serde_json::Map::new();
        if !self.config.system_sections.is_empty() {
            let sections: serde_json::Map<String, serde_json::Value> = self
                .config
                .system_sections
                .iter()
                .map(|(name, text)| (name.clone(), serde_json::Value::String(text.clone())))
                .collect();
            extra.insert("sections".to_string(), serde_json::Value::Object(sections));
            // **同一份数据的按 TS 键序文本**：事件载荷（`message_start`/`message_end` 里的
            // 这条 system 消息）也要保序——`serde_json::Map` 是 `BTreeMap`，一旦经它输出就成
            // 字母序（`cwd, docs, …`），而 TS 是 `preamble, …`。与 `session_writer` 的
            // 组装同源（那边一直有，这边此前漏了 ⇒ golden/S20c 的 B 层首差即此）。
            let mut raw = String::from("{");
            for (index, (name, text)) in self.config.system_sections.iter().enumerate() {
                if index > 0 {
                    raw.push(',');
                }
                raw.push_str(&serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string()));
                raw.push(':');
                raw.push_str(&serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string()));
            }
            raw.push('}');
            extra.insert(
                pi_ai::message::SECTIONS_RAW_KEY.to_string(),
                serde_json::Value::String(raw),
            );
        }
        if !self.tools.is_empty() {
            let current = tools_added_json(&self.tools);
            let (added, removed) = tool_state_changes(&self.config.system_tools_previous, &current);
            extra.insert("toolsAdded".to_string(), serde_json::Value::Array(added));
            if !removed.is_empty() {
                extra.insert(
                    "toolsRemoved".to_string(),
                    serde_json::Value::Array(removed),
                );
            }
            // 同一份数据的**按 TS 键序文本**：会话条目要用它（`Message::System` 的手写
            // `Serialize` 会把 `__toolsAddedRaw` 以 `RawValue` 原样写出）。
            extra.insert(
                pi_ai::message::TOOLS_ADDED_RAW_KEY.to_string(),
                serde_json::Value::String(tools_added_raw(&self.tools)),
            );
        }
        Some(Message::System {
            content: pi_ai::UserContent::Text(String::new()),
            timestamp: 0,
            extra,
        })
    }

    /// 请求一次并流式转发事件，返回最终助手消息。
    async fn stream_turn(
        &self,
        request: ProviderRequest,
        cancel: &CancellationToken,
    ) -> Result<pi_ai::AssistantMessage, LoopError> {
        // **进入本回合前就已取消就不发请求**：ref 的流函数在信号已中止时直接产出中止消息
        // （`openai-completions.ts:682-687`），只发一对 `message_start`/`message_end`。
        // golden/S09b 的第 19-22 条即此（`requests.jsonl` 仍只有 1 条）。
        // **位置必须在 `self.provider.stream(...)` 之前**——我第一版放在事件循环前，
        // 探测显示"入口 cancelled=true 却仍发了请求"，因为请求早就出去了。
        if cancel.is_cancelled() {
            // **注意与"流中途被中止"（S09a）不同**：这里是"带着已中止的信号去发请求"，
            // TS 的 `fetch` 会立刻抛 `AbortError`，于是走**错误物化**路径：
            // `stopReason: "error"` + `errorMessage: "This operation was aborted"`
            // （那是运行时抛出的原文，不是 `openai-completions.ts:682` 的 "Request was aborted"）。
            // golden/S09b 的 `message_start` **本身就带这两个字段**（流没开始过，消息是整体的）。
            let mut aborted = placeholder_assistant_with(&self.config);
            aborted.stop_reason = StopReason::Error;
            aborted.error_message = Some("This operation was aborted".to_string());
            let message = Message::Assistant(Box::new(aborted.clone()));
            self.sink.emit(AgentEvent::MessageStart {
                message: message.clone(),
            });
            self.sink.emit(AgentEvent::MessageEnd { message });
            return Ok(aborted);
        }
        // **provider 失败物化成一条 assistant 消息**，而不是把 `Err` 抛给调用方。
        //
        // 依据（批次 B5 的 D4 结论 + golden/S07 的 25 条事件）：405/429 这类失败在 TS 侧
        // 是一条 `stopReason: "error"` + `errorMessage` 的 assistant 消息，**该轮照常收尾**
        // （`message_start`/`message_end`/`turn_end`/`agent_end`），重试由产品层在其后发起。
        // 我原先直接上抛，导致失败那轮连事件都没有、包装器也看不到"可重试的失败"。
        let mut stream = match self.provider.stream(request).await {
            Ok(stream) => stream,
            Err(error) => {
                let text = match &error {
                    pi_ai::ProviderError::Http { message, .. } => message.clone(),
                    other => other.to_string(),
                };
                let message = pi_ai::AssistantMessage {
                    role: "assistant".to_string(),
                    content: Vec::new(),
                    api: self.config.api.clone(),
                    provider: self.config.provider.clone(),
                    model: self.config.model.clone(),
                    response_model: None,
                    usage: pi_ai::Usage::default(),
                    stop_reason: StopReason::Error,
                    error_message: Some(text),
                    raw_stop_reason: None,
                    end_turn: None,
                    response_id: None,
                    timestamp: 0,
                };
                self.sink.emit(AgentEvent::MessageStart {
                    message: Message::Assistant(Box::new(message.clone())),
                });
                self.sink.emit(AgentEvent::MessageEnd {
                    message: Message::Assistant(Box::new(message.clone())),
                });
                return Ok(message);
            }
        };
        let mut started = false;
        let mut last: Option<pi_ai::AssistantMessage> = None;
        loop {
            // **取消必须能中断"等下一个事件"本身**，不能只在事件之间检查：
            // S09a（abort）里闸门卡在服务端，`next_event()` 一直等不到数据，
            // 只检查 `is_cancelled()` 会**永远等下去**（实测：abort 侧等了 30 秒超时、
            // 而 run 侧仍卡着，两边互等——runner 在等 abort 的响应、run 在等闸门）。
            // `tokio::select!` 让取消一到就跳出，随后 `stream` 在本函数结束时被 drop，
            // 在途的 HTTP 请求随之取消（ref 的 `session.abort()` 也是这个效果：
            // 它 resolve 时流已经停了，所以响应排在整轮事件之后）。
            let next = tokio::select! {
                () = cancel.cancelled() => None,
                event = stream.next_event() => event,
            };
            let Some(event) = next else {
                break;
            };
            if cancel.is_cancelled() {
                break;
            }
            let assistant_event = event.clone();
            match event {
                AssistantEvent::Start => {
                    // 流式开始时先发一个初始快照，后续增量覆盖。
                    started = true;
                    self.sink.emit(AgentEvent::MessageStart {
                        message: Message::Assistant(Box::new(placeholder_assistant_with(
                            &self.config,
                        ))),
                    });
                }
                other => {
                    if !started {
                        started = true;
                        self.sink.emit(AgentEvent::MessageStart {
                            message: Message::Assistant(Box::new(placeholder_assistant_with(
                                &self.config,
                            ))),
                        });
                    }
                    let _ = other;
                }
            }
            // `Start` 不发 `message_update`：它已经由 `message_start` 表达。
            // golden/S01 的 `message_update` 序列是 text_start → text_delta×2 → text_end，
            // 我原先多发了首条 `{"assistantMessageEvent":{"type":"start"}}`。
            // `Start` 由 `message_start` 表达；`Done`/`Error` 由 `message_end` 表达
            // ——golden/S01 的 `message_update` 只有 text_start/delta/end 四类。
            if matches!(
                assistant_event,
                AssistantEvent::Start | AssistantEvent::Done { .. } | AssistantEvent::Error { .. }
            ) {
                continue;
            }
            // P4-15 步骤 5.1：中毒（provider 侧持锁 panic）**不能**被当成"还没产生终态"。
            let message = match stream.final_message() {
                Ok(Some(message)) => Some(message),
                Ok(None) => None,
                Err(poisoned) => {
                    // 状态已损坏：把原因写进可见的消息里并结束本轮，而不是继续假装正常。
                    let mut broken = placeholder_assistant_with(&self.config);
                    broken.stop_reason = StopReason::Error;
                    broken.error_message = Some(poisoned.to_string());
                    last = Some(broken.clone());
                    self.sink.emit(AgentEvent::MessageUpdate {
                        message: Message::Assistant(Box::new(broken)),
                        assistant_event: Some(assistant_event.clone()),
                    });
                    break;
                }
            };
            if let Some(message) = message {
                // **流式期间的快照 usage 必须是零**：TS 每个增量事件带的 `partial` 里 usage 全为 0，
                // 要等 `done`（OpenAI 的 `stream_options.include_usage` 把 usage 放最后一个 chunk）才填。
                //
                // 为什么放在这里而不是解码器里：我的解码器是**一次性**解完一个 chunk 的
                // （mock 的整段响应就是一个 chunk），进循环时 `finished` 已为真，
                // 因此按 `finished` 门控无效；而"按事件产出快照"需要解码器支持按事件回放，
                // 那是更大的改动。这里按**事件**决定，语义上等价于 TS 的 per-event 快照。
                // **不置零 usage**。我一度按"S01 单 turn 时流式 usage 全为 0"写了一条
                // "一律清零"的规则，S02 直接否掉它：golden 的 turn 1 在 `toolcall_start`
                // 就带 `in/out=20/8`，turn 2 到 `text_delta` 才出现 `10/2`。
                // 真实语义是"TS 的 `partial` 携带**当前已收到**的用量"——那是 per-event 快照，
                // 而我的解码器是**按 chunk 一次解完**，做不到按事件粒度。
                // 见 notes：这是已知限制，不是可以靠一条规则抹平的东西。
                last = Some(message.clone());
                self.sink.emit(AgentEvent::MessageUpdate {
                    message: Message::Assistant(Box::new(message)),
                    assistant_event: Some(assistant_event.clone()),
                });
            }
        }
        let mut final_message = match stream.final_message() {
            Ok(message) => message,
            Err(poisoned) => {
                // 中毒时**优先**给出带错误的占位（下面的 `.or(last)` 不会把它换掉）。
                let mut broken = placeholder_assistant_with(&self.config);
                broken.stop_reason = StopReason::Error;
                broken.error_message = Some(poisoned.to_string());
                Some(broken)
            }
        }
        .or(last)
        .unwrap_or_else(|| placeholder_assistant_with(&self.config));
        // **中止封口**：被取消时，最终消息要标成中止态并带上原因
        // （ref `openai-completions.ts:682-687,712`：`throw new Error("Request was aborted")`；
        // `output.stopReason = signal.aborted ? "aborted" : "error"`）。
        // golden/S09a 的 `message_end`/`turn_end` 里 `stopReason` 是 `"aborted"`、
        // `errorMessage` 是 `"Request was aborted"`；我此前保留 `pending` 且没有 errorMessage。
        if cancel.is_cancelled() {
            final_message.stop_reason = StopReason::Aborted;
            final_message.error_message = Some("Request was aborted".to_string());
        }
        // **兜底**：若整条流一个事件都没产出（S13b 的 length 截断夹具就是这样——参数不完整，
        // 解码器没有可发的事件），`started` 仍是假，于是此前**只发了 `message_end`**，
        // 少了配对的 `message_start`（B 层比对显示 21 vs 22 条）。
        // 每条 assistant 消息都必须先被 announce（`agent-loop.ts:417`）。
        if !started {
            // 用**占位**而不是 `final_message`：golden/S13b 的 `message_start` 不带
            // `errorMessage`（错误只出现在 `message_end`），与正常路径的 `Start` 分支一致。
            self.sink.emit(AgentEvent::MessageStart {
                message: Message::Assistant(Box::new(placeholder_assistant_with(&self.config))),
            });
        }
        self.sink.emit(AgentEvent::MessageEnd {
            message: Message::Assistant(Box::new(final_message.clone())),
        });
        // 重试的那一轮成功了：`auto_retry_end` 紧跟这条 `message_end`（golden/S07 实测）。
        if let Some(attempt) = self.config.retry_attempt {
            if !matches!(final_message.stop_reason, StopReason::Error) {
                self.sink.emit(AgentEvent::AutoRetryEnd {
                    success: true,
                    attempt,
                    final_error: None,
                });
            }
        }
        Ok(final_message)
    }

    /// 执行一批工具调用。
    ///
    /// **顺序**：`tool_execution_start` 按源码顺序发出；`tool_execution_end` 按**完成顺序**
    /// 发出（每个执行体自己发，谁先完成谁先发）；返回的工具结果消息再按**源码顺序**排列
    /// ——与 `agent-loop.ts:616-651` 一致。
    async fn run_batch(
        &self,
        assistant: &pi_ai::AssistantMessage,
        tool_calls: &[(String, String, Value)],
        stop_reason: StopReason,
        cancel: &CancellationToken,
    ) -> (Vec<Message>, bool) {
        // `stopReason == length` 时**一律不执行**，统一报错（`agent-loop.ts:468-499`）。
        if let Some(_first) = tool_calls.first() {
            if matches!(stop_reason, StopReason::Length) {
                let results: Vec<Message> = tool_calls
                    .iter()
                    .map(|(id, name, _)| {
                        let text = length_limit_error(&stop_reason, name).unwrap_or_default();
                        // **先发 `tool_execution_start` 再发 end**：golden/S13b-toolcall 实测
                        // 这两个事件成对出现（我为 length 路径只发了 end，少一条；
                        // 由 B 层比对指出）。
                        self.sink.emit(AgentEvent::ToolExecutionStart {
                            tool_call_id: id.clone(),
                            tool_name: name.clone(),
                            args: serde_json::Value::Object(serde_json::Map::new()),
                        });
                        // 结果形状与正常路径一致：`{content:[…], details:{}}`（出错时 details 是空对象）。
                        let result = crate::tools::tool::ToolResult::error(text.clone());
                        self.sink.emit(AgentEvent::ToolExecutionEnd {
                            tool_call_id: id.clone(),
                            tool_name: name.clone(),
                            result: tool_result_json(&result),
                            is_error: true,
                        });
                        let blocks =
                            [crate::tools::tool::ToolResultBlock::Text { text: text.clone() }];
                        tool_result_message(id, name, &blocks, true, result.details.as_ref())
                    })
                    .collect();
                return (results, false);
            }
        }

        let modes: Vec<ExecutionMode> = tool_calls
            .iter()
            .map(|_| self.config.tool_execution)
            .collect();
        let mode = batch_execution_mode(self.config.tool_execution, &modes);
        let outcomes = aborted_call_outcomes(tool_calls.len(), 0, cancel.is_cancelled());

        let mut futures = Vec::with_capacity(tool_calls.len());
        for ((id, name, args), aborted) in tool_calls.iter().zip(outcomes) {
            // **开段**：把 `tool_execution_start` 与工具的首条进度上报绑成"其他线程不得插入"的一段
            // （P4-15 步骤 1；见 `agent/section.rs` 与 S10 的顺序竞态）。
            self.sink
                .emit_opening_section(AgentEvent::ToolExecutionStart {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    args: args.clone(),
                });
            let id = id.clone();
            let name = name.clone();
            let args = args.clone();
            // 每个执行体**自己**在完成时发出 `tool_execution_end`——这样事件顺序天然是
            // **完成顺序**（`agent-loop.ts:616-625` 把 emit 写在并发 thunk 内部）。
            // 若改成"等 `join_all` 之后再按源码顺序发"，就与 TS 不一致了（这条差异是被
            // `t3_25_agent_loop::event_order_and_parallel_completion_order_match_the_spec` 抓出来的）。
            let end_sink = self.sink.clone();
            futures.push(async move {
                let result = if let Some(text) = aborted {
                    ToolResult {
                        content: vec![crate::tools::tool::ToolResultBlock::Text { text }],
                        is_error: true,
                        details: None,
                        terminate: false,
                    }
                } else {
                    match self.tools.iter().find(|tool| tool.name() == name) {
                        Some(tool) => {
                            // 工具执行期间的进度上报（对应 TS 的 `onUpdate`）：每次上报发一条
                            // `tool_execution_update`（golden/S05 实测一次调用会有多条）。
                            let update_sink = end_sink.clone();
                            let update_id = id.clone();
                            let update_name = name.clone();
                            let update_args = args.clone();
                            let context = ToolContext {
                                cwd: self.cwd.clone(),
                            };
                            let on_update = move |partial: &ToolResult| {
                                update_sink.emit(AgentEvent::ToolExecutionUpdate {
                                    tool_call_id: update_id.clone(),
                                    tool_name: update_name.clone(),
                                    args: update_args.clone(),
                                    partial: tool_result_json(partial),
                                });
                            };
                            tool.invoke_with_progress(args, &context, cancel, &on_update)
                                .await
                        }
                        None => ToolResult {
                            content: vec![crate::tools::tool::ToolResultBlock::Text {
                                text: format!("Unknown tool: {name}"),
                            }],
                            is_error: true,
                            details: None,
                            terminate: false,
                        },
                    }
                };
                end_sink.emit(AgentEvent::ToolExecutionEnd {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    result: tool_result_json(&result),
                    is_error: result.is_error,
                });
                (id, name, result)
            });
        }
        let _ = mode;
        let finished = join_all(futures).await;
        let mut results = Vec::with_capacity(finished.len());
        let mut terminates = Vec::with_capacity(finished.len());
        for (id, name, result) in &finished {
            // 工具结果**消息**在这里按 `join_all` 保序的源码顺序产出；`tool_execution_end`
            // 事件已经由各执行体在完成时发过了，**不能**在这里再发一次。
            results.push(tool_result_message(
                id,
                name,
                &result.content,
                result.is_error,
                // 工具结果**消息**要带 `details`（golden/S04 的 `message_start` 载荷里有它）。
                result.details.as_ref(),
            ));
            terminates.push(result.terminate);
        }
        let _ = assistant;
        let _ = mode;
        (results, should_terminate_batch(&terminates))
    }

    /// 请求体里的工具列表。**顺序即请求体顺序**（`provider.rs:30-31` 的约定），
    /// 因此按注册顺序给出，不排序。
    fn tool_specs(&self) -> Vec<pi_ai::openai::ToolSpec> {
        self.tools
            .iter()
            .map(|tool| pi_ai::openai::ToolSpec {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.schema().clone(),
            })
            .collect()
    }
}

/// 便捷入口：跑一次循环。
pub async fn run_loop(
    agent_loop: &AgentLoop,
    messages: Vec<Message>,
    cancel: &CancellationToken,
) -> Result<Vec<Message>, LoopError> {
    agent_loop
        .run(messages, &|_| Vec::new(), &|_| Vec::new(), cancel)
        .await
}

/// `tool_execution_end.result` 的线上形状。
///
/// golden/S03 实测是 `{"content":[{"type":"text","text":…}]}`——即工具结果的内容块数组
/// （我原先发的是自造的 `{"text":…}`）。
/// **图片块的形状未验证**：会话里是 `{type:"image",data,mimeType}`，这里照此拼。
fn tool_result_json(result: &crate::tools::tool::ToolResult) -> serde_json::Value {
    let blocks: Vec<serde_json::Value> = result
        .content
        .iter()
        .map(|block| match block {
            crate::tools::tool::ToolResultBlock::Text { text } => {
                serde_json::json!({ "type": "text", "text": text })
            }
            crate::tools::tool::ToolResultBlock::Image { data, mime_type } => {
                serde_json::json!({ "type": "image", "data": data, "mimeType": mime_type })
            }
        })
        .collect();
    let mut value = serde_json::json!({ "content": blocks });
    // `details` 的三种形态（都来自 golden 实测）：
    // - 成功且工具有 details（edit）→ 带该对象（S04）；
    // - 成功且工具无 details（write）→ **不带该键**（S03）；
    // - 出错 → 带**空对象** `{}`（S26/S04b/S06 三条实测）。
    match &result.details {
        Some(details) => value["details"] = details.clone(),
        None if result.is_error => value["details"] = serde_json::json!({}),
        None => {}
    }
    value
}

/// `toolsAdded` 的线上形状：`{name, description, parameters, constrainedSampling}`。
///
/// 事件（`message_start.message.toolsAdded`）与会话条目（`session.jsonl` 的 system 那条）
/// **用的是同一个形状**（golden/S01 两处实测一致），所以抽成一个函数两边共用——
/// 我先前在会话侧只写了 `{name, description}`，逐行核对立刻显出差异。
/// `toolsAdded` 的**按 TS 键序的文本**（用于会话条目：TS 的 `declarationsEqual` 用
/// `JSON.stringify` 判等，`parameters` 的键序参与判等——见 `tools/schema_order.rs`）。
///
/// **不变量**：它与 [`tools_added_json`] **语义相同**、只有键序不同。
pub fn tools_added_raw(tools: &[std::sync::Arc<dyn crate::tools::tool::Tool>]) -> String {
    let mut out = String::from("[");
    for (index, tool) in tools.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        // 键序：`name, description, parameters, constrainedSampling`（照 TS 的 `toToolDeclaration`）。
        out.push_str("{\"name\":");
        out.push_str(&serde_json::to_string(tool.name()).unwrap_or_else(|_| "\"\"".to_string()));
        out.push_str(",\"description\":");
        out.push_str(
            &serde_json::to_string(tool.description()).unwrap_or_else(|_| "\"\"".to_string()),
        );
        out.push_str(",\"parameters\":");
        out.push_str(&crate::tools::schema_order::render_schema_for(
            tool.name(),
            tool.schema(),
        ));
        out.push_str(",\"constrainedSampling\":{\"type\":\"json_schema\",\"strict\":\"prefer\"}}");
    }
    out.push(']');
    out
}

/// ref `packages/ai/src/utils/transcript.ts:150-167` 的 `getToolStateChanges`。
///
/// - `toolsAdded`：当前工具里"上一份没有**或声明变了**"的（声明变化也算新增）；
/// - `toolsRemoved`：上一份工具里"当前没有**或声明变了**"的，只取 `{name}`。
///
/// `declarationsEqual` 照 ref `:140-141` 用 `JSON.stringify` 比对 ⇒ **键序参与判等**
/// （这是有意的：键序不同即视为声明变化；本函数因此用 `Value::to_string()` 而不是比较解析后的值）。
#[must_use]
pub fn tool_state_changes(
    previous: &[serde_json::Value],
    current: &[serde_json::Value],
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let name_of = |item: &serde_json::Value| {
        item.get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let find = |list: &[serde_json::Value], name: &str| {
        list.iter()
            .find(|item| name_of(item) == name)
            .map(serde_json::Value::to_string)
    };
    let added = current
        .iter()
        .filter(|tool| match find(previous, &name_of(tool)) {
            None => true,
            Some(previous_text) => previous_text != tool.to_string(),
        })
        .cloned()
        .collect();
    let removed = previous
        .iter()
        .filter(|tool| match find(current, &name_of(tool)) {
            None => true,
            Some(current_text) => current_text != tool.to_string(),
        })
        .map(|tool| serde_json::json!({ "name": name_of(tool) }))
        .collect();
    (added, removed)
}

pub fn tools_added_json(
    tools: &[std::sync::Arc<dyn crate::tools::tool::Tool>],
) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name(),
                "description": tool.description(),
                "parameters": tool.schema(),
                "constrainedSampling": { "type": "json_schema", "strict": "prefer" },
            })
        })
        .collect()
}

/// 把 `UserContent::Text` 规范成 `Blocks([Text])`；其余原样。
fn normalize_user_content(message: Message) -> Message {
    match message {
        Message::User {
            content,
            timestamp,
            extra,
        } => {
            let content = match content {
                pi_ai::UserContent::Text(text) => {
                    pi_ai::UserContent::Blocks(vec![pi_ai::Content::Text {
                        text,
                        text_signature: None,
                    }])
                }
                pi_ai::UserContent::Blocks(blocks) => pi_ai::UserContent::Blocks(blocks),
            };
            Message::User {
                content,
                timestamp,
                extra,
            }
        }
        other => other,
    }
}

/// 取 assistant 消息里的工具调用，顺序即源码顺序。
fn tool_calls_of(message: &pi_ai::AssistantMessage) -> Vec<(String, String, Value)> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            pi_ai::Content::ToolCall {
                id,
                name,
                arguments,
                ..
            } => Some((id.clone(), name.clone(), Value::Object(arguments.clone()))),
            pi_ai::Content::Text { .. }
            | pi_ai::Content::Thinking { .. }
            | pi_ai::Content::Image { .. } => None,
        })
        .collect()
}

/// 流式开始时的占位 assistant 消息（字段留空，后续增量覆盖）。
fn placeholder_assistant_with(config: &LoopConfig) -> pi_ai::AssistantMessage {
    pi_ai::AssistantMessage {
        role: "assistant".to_string(),
        content: Vec::new(),
        // golden/S01 的 `message_start` 带真实的 api/provider/model，`stopReason` 是 `pending`
        // （我原先留空并写 `stop`，被 B 层比对指出）。
        api: config.api.clone(),
        provider: config.provider.clone(),
        model: config.model.clone(),
        response_model: None,
        usage: pi_ai::Usage::default(),
        stop_reason: StopReason::Pending,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 0,
    }
}

/// 工具结果消息。`details` 也要带——golden/S04 的 `message_start` 里
/// toolResult 消息同时有 `content` 与 `details`（我原先只放了 content）。
fn tool_result_message(
    id: &str,
    name: &str,
    blocks: &[crate::tools::tool::ToolResultBlock],
    is_error: bool,
    details: Option<&serde_json::Value>,
) -> Message {
    let mut extra = serde_json::Map::new();
    match details {
        Some(details) => {
            extra.insert("details".to_string(), details.clone());
        }
        // 出错时消息里也带空对象（与事件侧同一形态，golden/S26 实测）。
        None if is_error => {
            extra.insert("details".to_string(), serde_json::json!({}));
        }
        None => {}
    }
    Message::ToolResult {
        tool_call_id: id.to_string(),
        tool_name: name.to_string(),
        // P(b)：按**原块**构造，Text 与 Image 都保留（此前经 `text_content()` 折叠成单条 Text，
        // 图片块在会话里丢失；ref 的 tool 消息内容是 `(TextContent | ImageContent)[]`）。
        content: blocks
            .iter()
            .map(|block| match block {
                crate::tools::tool::ToolResultBlock::Text { text } => {
                    pi_ai::ToolResultContent::Text {
                        text: text.clone(),
                        extra: serde_json::Map::new(),
                    }
                }
                crate::tools::tool::ToolResultBlock::Image { data, mime_type } => {
                    pi_ai::ToolResultContent::Image {
                        data: data.clone(),
                        mime_type: mime_type.clone(),
                        extra: serde_json::Map::new(),
                    }
                }
            })
            .collect(),
        is_error,
        timestamp: 0,
        extra,
    }
}
