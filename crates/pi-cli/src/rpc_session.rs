//! rpc 的会话运行时：命令派发 + agent loop。
//!
//! **并发模型**（这是 S09b/S10/S11 的前提）：
//! - 主线程**只负责读 stdin 与派发**，`prompt` 立刻回响应后在**后台线程**跑；
//! - 后台线程与主线程共享：两个 `PendingQueue`（steer/follow_up）、取消令牌、
//!   消息账本、以及**一个加锁的 stdout**——事件与响应都经它写出，避免行交错；
//! - `abort` 取消当前运行的令牌；`steer`/`follow_up` 入队，由循环在自己的 turn 边界取
//!   （取法早已实现，见 `AgentLoop::run` 的两个回调）。
//!
//! 为什么必须这样：rpc 场景的脚本是"等到事件 → 发命令 → 继续等"，单线程顺序处理
//! stdin 会让运行期间的命令读不到，场景直接死等超时（S09b/S10/S11 就是这么被卡住的）。

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pi_ai::message::Message;
use pi_ai::Provider;
use pi_core::agent::{AgentEvent, EventSink, LoopConfig, PendingQueue};
use pi_core::tools::tool::Tool;

use crate::rpc::{handle_line, RpcRuntime, RpcSessionState};

/// 主线程与后台运行线程共享的状态。
pub struct RpcShared {
    pub provider: Arc<dyn Provider>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub cwd: PathBuf,
    /// agent 配置目录（`get_commands` 的 `sourceInfo.baseDir` 用它，见 golden/S24b）。
    pub agent_dir: PathBuf,
    pub model: String,
    pub system: Option<String>,
    /// 组装出的系统提示段落（与 print 路径同源）；写会话的 system 条目要用它。
    pub sections: Vec<(String, String)>,
    /// 解析出的 api 标识（如 `openai-completions`）。**必须与 print 同源**：
    /// 此前 rpc 用 `api: String::new()`，事件里的 assistant 消息 `api` 是空串（S09a 的 B 层指出）。
    pub api: String,
    /// 已加载的技能（`/skill:` 展开用；ref `agent-session.ts:1649/1841` 与 print 共用）。
    pub skills: Vec<pi_core::skills::Skill>,
    /// 已加载的 prompt template（`/名字` 展开用；ref `agent-session.ts:1646-1651` 与 print 共用，
    /// 顺序为「先 `/skill:`、后模板」）。
    pub prompt_templates: Vec<pi_core::prompt_templates::PromptTemplate>,
    /// 是否推理模型（决定初始思考等级）。
    pub reasoning: bool,
    /// 当前思考等级（`set_thinking_level` 会改它；`get_state` 会报它）。
    pub level: std::sync::Mutex<String>,
    /// `set_model` 的响应 `data`：**完整的模型对象**（golden/S16 实测含
    /// `id/name/api/provider/baseUrl/reasoning/input/cost/contextWindow/maxTokens`）。
    pub model_json: serde_json::Value,
    /// system 条目是否已写。**命令期建会话时不写**（golden/S16 的顺序），
    /// 首次运行时再写一次；所以需要这个标志避免重复。
    pub system_written: std::sync::Mutex<bool>,
    /// 组装出的系统提示段落（`LoopConfig` 用，决定 `message_start` 里的 `sections`）。
    pub system_sections: std::collections::BTreeMap<String, String>,
    pub max_tokens: u64,
    pub provider_name: String,
    /// `settings.json` 的 `compaction` 段（已与默认值合并）；手动/自动压缩都要它。
    pub settings: pi_core::agent::compaction::CompactionSettings,
    /// 模型的上下文窗口（压缩阈值判定要用）。
    pub context_window: u64,
    pub model_id: String,
    pub steering: Arc<PendingQueue>,
    pub follow_up: Arc<PendingQueue>,
    /// 当前运行的取消令牌；每次运行换一个新的。
    pub cancel: Mutex<tokio_util::sync::CancellationToken>,
    /// 已处理的消息（`get_state.messageCount`）。
    pub transcript: Mutex<Vec<Message>>,
    /// 是否有运行在进行（`get_state.isStreaming`）。
    pub busy: AtomicBool,
    /// 加锁的 stdout：事件行与响应行都从这里出，保证不交错。
    pub out: Mutex<Box<dyn Write + Send>>,
    /// 事件流的原子段闸门（P4-15 步骤 1）：agent 线程发出 `tool_execution_start` 后开段，
    /// 本线程的下一条事件（工具的首条进度上报）之后自动关段；期间**其他线程**（即 RPC 线程）
    /// 的写要等它结束。复刻 TS"RPC 命令处理只能发生在 agent await 点"的语义。
    pub section: pi_core::agent::section::SectionGate,
    pub runtime: Arc<tokio::runtime::Runtime>,
    /// 会话写入器（懒创建；`--no-session` 时为 `None` 且 `session_dir` 也为 `None`）。
    pub session: Mutex<Option<crate::session_writer::SessionWriter>>,
    pub session_dir: Option<PathBuf>,
    /// **启动时**固定的"当前会话文件"路径（`--continue`/`--resume` 载入的那份）。
    ///
    /// 为什么必须固定：`newest_session_file` 取目录里**字典序最大**的 `.jsonl`，而本进程
    /// 一建会话就会写出以时间戳开头的新文件（`2026-…` 排在 `seed.jsonl` 之后）⇒ 之后再去
    /// "找最新"就找不到 `--continue` 载入的种子了（S20c：事件侧 `toolsRemoved` 缺失即此）。
    pub loaded_session_path: Option<PathBuf>,
    /// **预生成**的会话（iso, id, path）；`--no-session` 时为 `None`。
    ///
    /// 与 TS 对齐：会话 id 与文件名在**启动时**确定，文件在首次写入时才创建——
    /// 所以 `get_state` 在写器还没建时也要报出它们（S35 暴露过这里的差异）。
    pub precomputed_session: Option<(String, String, std::path::PathBuf)>,
    /// 本轮的 system 消息是否要**当作新消息 announce**。
    ///
    /// 与 print 路径同规则：只有"会话**已存在**且里面没有 system"时为真（`--continue` 或
    /// `fork` 之后）。新建会话的首轮**不** announce system（TS 也是）。
    pub announce_system: std::sync::atomic::AtomicBool,
    /// 已经写进会话文件的消息条数（避免重复追加）。
    pub written: Mutex<usize>,
}

impl RpcShared {
    /// 拿到（必要时创建）会话写入器并执行 `f`。`--no-session` 时返回 `None`。
    ///
    /// 抽出它是为了让 `set_thinking_level`/`set_model` 也能写条目——
    /// 这两条命令可能在**第一次运行之前**就被调用（golden/S16 就是先切换再 prompt）。
    fn with_writer<R>(
        &self,
        f: impl FnOnce(&mut crate::session_writer::SessionWriter) -> R,
    ) -> Option<R> {
        let dir = self.session_dir.as_deref()?;
        let mut guard = self.session.lock().ok()?;
        if guard.is_none() {
            let thinking = if self.reasoning { "medium" } else { "off" };
            let writer = match &self.precomputed_session {
                Some((iso, id, path)) => crate::session_writer::SessionWriter::create_precomputed(
                    dir,
                    &self.cwd,
                    &self.provider_name,
                    &self.model_id,
                    thinking,
                    iso,
                    id,
                    path,
                )
                .ok()?,
                None => crate::session_writer::SessionWriter::create(
                    dir,
                    &self.cwd,
                    &self.provider_name,
                    &self.model_id,
                    thinking,
                )
                .ok()?,
            };
            // **不在这里写 system 条目**：命令期只建会话；system 由首次运行时
            // `write_new_messages` 写（golden/S16 的顺序是 …切换条目、system、user…）。
            *guard = Some(writer);
        }
        guard.as_mut().map(f)
    }

    /// 把本次运行新增的消息追加进会话文件。
    ///
    /// rpc 与 print 都必须落盘，否则差分器的 A 层永远是空的（这条是 S09b 暴露出来的）。
    fn write_new_messages(&self, transcript: &[Message]) {
        let Some(dir) = self.session_dir.as_deref() else {
            return;
        };
        let mut guard = match self.session.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if guard.is_none() {
            let thinking = if self.reasoning { "medium" } else { "off" };
            let created = match &self.precomputed_session {
                Some((iso, id, path)) => crate::session_writer::SessionWriter::create_precomputed(
                    dir,
                    &self.cwd,
                    &self.provider_name,
                    &self.model_id,
                    thinking,
                    iso,
                    id,
                    path,
                ),
                None => crate::session_writer::SessionWriter::create(
                    dir,
                    &self.cwd,
                    &self.provider_name,
                    &self.model_id,
                    thinking,
                ),
            };
            match created {
                Ok(writer) => {
                    if self.system.is_some() {
                        let mut writer = writer;
                        // 与 print 路径**同源**：组装的 sections + 四键 `toolsAdded`。
                        let tools_added = pi_core::agent::tools_added_json(&self.tools);
                        let tools_added_raw = pi_core::agent::tools_added_raw(&self.tools);
                        if let Err(error) = writer.append_system_message(
                            &self.sections,
                            &tools_added,
                            &tools_added_raw,
                        ) {
                            eprintln!("pi-rs: 写 system 消息失败：{error}");
                        }
                        // **置位标志**：否则下面"命令期没写则补写"的分支会再写一遍
                        // （S09a 的 A 层立刻多出一条 message，6 → 7 行）。
                        if let Ok(mut written) = self.system_written.lock() {
                            *written = true;
                        }
                        *guard = Some(writer);
                    } else {
                        *guard = Some(writer);
                    }
                }
                Err(error) => {
                    eprintln!("pi-rs: 建会话失败：{error}");
                    return;
                }
            }
        }
        // **会话可能是命令期（`set_thinking_level`/`set_model`）已建好的**，
        // 那时没写 system 条目；这里补一次，并用标志防重。
        if self.system.is_some() {
            let mut need_system = false;
            if let Ok(mut written) = self.system_written.lock() {
                if !*written {
                    *written = true;
                    need_system = true;
                }
            }
            if need_system {
                if let Some(writer) = guard.as_mut() {
                    let tools_added = pi_core::agent::tools_added_json(&self.tools);
                    let tools_added_raw = pi_core::agent::tools_added_raw(&self.tools);
                    if let Err(error) =
                        writer.append_system_message(&self.sections, &tools_added, &tools_added_raw)
                    {
                        eprintln!("pi-rs: 写 system 消息失败：{error}");
                    }
                }
            }
        }
        let already = self.written.lock().map(|guard| *guard).unwrap_or_default();
        if let Some(writer) = guard.as_mut() {
            for message in transcript.iter().skip(already) {
                if let Err(error) = writer.append_message(message) {
                    eprintln!("pi-rs: 写会话失败：{error}");
                    break;
                }
            }
        }
        let _ = &mut guard;
        if let Ok(mut guard) = self.written.lock() {
            *guard = transcript.len();
        }
    }

    /// 发一条 `queue_update`（`agent-session.ts:837-843` 的 `_emitQueueUpdate`）。
    pub fn emit_queue_update(&self) {
        self.write_line(&queue_update_json(
            &self.steering.texts(),
            &self.follow_up.texts(),
        ));
    }

    /// 写一行到 stdout（加锁 + flush）。
    ///
    /// 写之前先过原子段闸门：agent 线程正处于"`tool_execution_start` → 工具首条进度"之间时，
    /// 本线程在这里等（最长安全阀时间），避免把 `queue_update` 插进那一对事件里
    /// （S10 的顺序竞态，P4-15 步骤 1）。
    pub fn write_line(&self, line: &str) {
        if !self.section.wait() {
            // 安全阀放行（P4-15 步骤 2.5）：有线程持有原子段超过时限（工具卡住、或首次上报迟迟不来）。
            // 只记 stderr，不污染 stdout 的事件流；计数在 `SectionGate::valve_releases` 里可观测。
            eprintln!(
                "pi-rs: 事件流原子段超过安全阀时限，已放行写入（累计 {} 次）",
                self.section.valve_releases()
            );
        }
        if let Ok(mut out) = self.out.lock() {
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
    }

    /// 跑一次 prompt（在后台线程里调用；阻塞到本次运行结束）。
    fn run_prompt(self: Arc<Self>, message: String) {
        self.busy.store(true, Ordering::SeqCst);
        let token = tokio_util::sync::CancellationToken::new();
        if let Ok(mut guard) = self.cancel.lock() {
            *guard = token.clone();
        }
        let mut agent_loop = pi_core::agent::AgentLoop::new(
            self.provider.clone(),
            self.tools.clone(),
            Arc::new(SharedSink(self.clone())),
            LoopConfig {
                model: self.model.clone(),
                api: self.api.clone(),
                provider: self.provider_name.clone(),
                system: self.system.clone(),
                system_sections: self.system_sections.clone(),
                instruction_role: if self.reasoning {
                    Some("developer".to_string())
                } else {
                    None
                },
                // **实时等级**（`set_thinking_level` 可能已改过）。
                reasoning_effort: if self.reasoning {
                    self.level.lock().ok().map(|guard| guard.clone())
                } else {
                    None
                },
                max_completion_tokens: self.max_tokens,
                announce_system: self
                    .announce_system
                    .load(std::sync::atomic::Ordering::Relaxed),
                // `toolsRemoved` 的差分依据：会话里最后一份已声明的工具（含 compaction 的
                // `systemMessage`）。读不到会话时为空 = 没有上一份。
                // **按 `--continue` 的同一规则取"当前会话"**：会话目录里最新的 `.jsonl`。
                // 不能用 `self.session`（写入器可能还没建）、也不能用 `with_writer`（它会
                // **新建**会话，读不到 `--continue` 载入的种子）——S20c 实测：会话文件里已有
                // `toolsRemoved`，事件载荷却没有，就是这两处取错了 `previous`。
                system_tools_previous: self
                    .loaded_session_path
                    .as_deref()
                    .and_then(|path| pi_core::session_io::SessionFile::read(path).ok())
                    .map(|file| pi_core::agent::last_declared_tools(&file.entries))
                    .unwrap_or_default(),
                ..LoopConfig::default()
            },
        );
        agent_loop.cwd = self.cwd.clone();
        // **提示词前的压缩检查**（ref `agent-session.ts:1693-1698`，`skipAbortedCheck=false`；
        // 回合后那次是 `:1522`。两次共用同一份判定与执行，不另写一套）。
        // （提示词前检查已上移到命令处理内、写 ack 之前——见 `RuntimeBridge::prompt`。）
        let mut messages = self
            .transcript
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        // **只 announce 本轮的消息**：`announce_from` 是既有上下文的长度。
        // print 路径早就有这一句；rpc 路径原来没设（transcript 为空时 `0` 恰好也对，
        // 但 `--continue`/`fork` 之后上下文非空，会把旧消息当新消息发 `message_start`
        // ——S28 的 B 层第 10 行就是这么暴露的）。
        agent_loop.set_announce_from(messages.len());
        messages.push(Message::User {
            content: pi_ai::UserContent::Text(message),
            timestamp: 0,
            extra: serde_json::Map::new(),
        });

        let steering = self.steering.clone();
        let follow_up = self.follow_up.clone();
        // 消费后也要发 `queue_update`（ref `agent-session.ts:905/911`：从队列里移除该条时）。
        // golden/S10 的第 2 条 `queue_update` 就在 `turn_start` 之后、被 steer 那条消息的
        // `message_start` 之前——即**消费时**。闭包在循环里被调用，`write_line` 与其它事件
        // 共用同一把 stdout 锁，因此顺序自然正确。
        // 两个闭包各持一份 `Arc`（`Fn` 闭包会被多次调用，共享一份会 move 冲突）。
        let shared_for_steering = self.clone();
        let shared_for_follow_up = self.clone();
        match self.runtime.block_on(agent_loop.run(
            messages,
            // 消费后的事件由**循环**在下一回合的 `turn_start` 之后发出（顺序要求），
            // 这里只负责取出消息。
            &move |mode| {
                let _ = &shared_for_steering;
                steering.drain(mode)
            },
            &move |mode| {
                let _ = &shared_for_follow_up;
                follow_up.drain(mode)
            },
            &token,
        )) {
            Ok(transcript) => {
                self.write_new_messages(&transcript);
                // **不要把 system 塞进这份 transcript**：它同时是下一轮循环的输入，
                // 塞进去会让请求体多一条 system（S20b 的 C 层立刻转红）。计数改在 `state()`
                // 读侧补（见 `message_count`），与 TS 的 `agent.state.messages` 对齐。
                if let Ok(mut guard) = self.transcript.lock() {
                    *guard = transcript;
                }
            }
            Err(error) => {
                // 循环抛错：按响应形状报一条 error（`id` 缺省，因为原命令已经回过响应了）。
                self.write_line(
                    &crate::rpc::response_error(None, "prompt", &error.to_string()).to_string(),
                );
            }
        }
        // P5-4a：**回合结束后的自动压缩**（与 print 路径同一套判定；ref `agent-session.ts:1522`
        // 的 `_checkCompaction(message, true, toolResults)`）。S20 的 golden 第 13 行就是它
        // （`compaction_start{reason:"overflow"}` → `compaction_end` → `agent_settled`）。
        self.maybe_auto_compact(&crate::auto_compaction::LineSink(std::sync::Arc::clone(
            &self,
        )));
        // 一次 run 结束的收尾事件（`agent-session.ts:171` 的 `{ type: "agent_settled" }`）。
        // runner 的 rpc 完成判据就是"等到 N 条 agent_settled"，缺了它会一直等到超时。
        self.write_line("{\"type\":\"agent_settled\"}");
        self.busy.store(false, Ordering::SeqCst);
    }
}

impl RpcShared {
    /// 提示词**之前**的压缩检查（ref `agent-session.ts:1693-1698`）。
    ///
    /// 与回合后那次（`:1522`）共用 `overflow_outcome`/`threshold_outcome` 与
    /// `perform_auto_compaction`；区别只有 `skip_aborted_check`（这里传 `false`：被中止的回合
    /// 不会走到回合后检查，所以要在提示词前接住它）。
    fn maybe_pre_prompt_compact(&self, sink: &dyn pi_core::agent::EventSink) {
        let Some(path) = self.loaded_session_path.clone() else {
            return;
        };
        let Ok(file) = pi_core::session_io::SessionFile::read(&path) else {
            return;
        };
        let assistant = file.entries.iter().rev().find_map(|entry| {
            if let pi_core::session::SessionEntry::Message {
                message: Message::Assistant(assistant),
                ..
            } = entry
            {
                Some((**assistant).clone())
            } else {
                None
            }
        });
        let Some(assistant) = assistant else {
            return;
        };
        let config = crate::auto_compaction::CompactionConfig {
            settings: self.settings,
            context_window: self.context_window,
            model: self.model_id.clone(),
            model_provider: self.provider_name.clone(),
            model_max_tokens: self.max_tokens,
            model_reasoning: self.reasoning,
            thinking_level: self.level.lock().ok().map(|level| level.clone()),
        };
        let Some(plan) =
            crate::auto_compaction::auto_compaction_plan(&file.entries, &config, &assistant, false)
        else {
            return;
        };
        self.write_line(
            &serde_json::json!({"type": "compaction_start", "reason": plan.reason}).to_string(),
        );
        let provider = std::sync::Arc::clone(&self.provider);
        let runtime = std::sync::Arc::clone(&self.runtime);
        let outcome = {
            let Ok(mut guard) = self.session.lock() else {
                return;
            };
            let Some(writer) = guard.as_mut() else {
                return;
            };
            runtime.block_on(crate::auto_compaction::perform_auto_compaction(
                &file.entries,
                &config,
                None,
                Some(plan.reason),
                Some(sink),
                provider.as_ref(),
                writer,
            ))
        };
        match outcome {
            Ok(Some(run)) => {
                // **压缩后的上下文要交给本轮**（与 print 路径同规则）：否则提示词请求里仍是
                // 压缩前的消息（golden/S13g 的 C 层首差即此）。`written` 同步成新条数。
                if let Ok(mut guard) = self.transcript.lock() {
                    *guard = run.messages.clone();
                }
                // **注意语义差异**：rpc 路径的 `written` = `transcript.len()`（见本函数末尾），
                // **不是** print 路径的 `1 + initial_messages.len()`（那里 +1 是 system 条目）。
                // 我第一版按 print 的语义写了 `1 +`，于是跳过一条、A/B 都对不上。
                if let Ok(mut guard) = self.written.lock() {
                    *guard = run.messages.len();
                }
                self.write_line(
                    &serde_json::json!({
                        "type": "compaction_end", "reason": plan.reason,
                        "result": run.result_event, "aborted": false, "willRetry": plan.will_retry,
                    })
                    .to_string(),
                );
            }
            Ok(None) => {}
            Err(error) => self.write_line(
                &serde_json::json!({
                    "type": "compaction_end", "reason": plan.reason, "aborted": false,
                    "willRetry": false, "errorMessage": error,
                })
                .to_string(),
            ),
        }
    }

    /// 回合结束后的自动压缩（print 路径的同名逻辑见 `print_mode`；判定与摘要调用都复用
    /// `crate::auto_compaction`）。写 `compaction_start`/`compaction_end` 事件，不改 rpc 响应。
    fn maybe_auto_compact(&self, sink: &dyn pi_core::agent::EventSink) {
        let Some(assistant) = self.transcript.lock().ok().and_then(|guard| {
            guard.iter().rev().find_map(|message| {
                if let Message::Assistant(assistant) = message {
                    Some(assistant.clone())
                } else {
                    None
                }
            })
        }) else {
            return;
        };
        let config = crate::auto_compaction::CompactionConfig {
            settings: self.settings,
            context_window: self.context_window,
            model: self.model_id.clone(),
            model_provider: self.provider_name.clone(),
            model_max_tokens: self.max_tokens,
            model_reasoning: self.reasoning,
            thinking_level: self.level.lock().ok().map(|level| level.clone()),
        };
        let Ok(mut guard) = self.session.lock() else {
            return;
        };
        let Some(writer) = guard.as_mut() else {
            return;
        };
        let Ok(file) = pi_core::session_io::SessionFile::read(writer.path()) else {
            return;
        };
        let entries = file.entries.clone();
        let Some(plan) =
            crate::auto_compaction::auto_compaction_plan(&entries, &config, &assistant, true)
        else {
            return;
        };
        self.write_line(
            &serde_json::json!({"type": "compaction_start", "reason": plan.reason}).to_string(),
        );
        match self
            .runtime
            .block_on(crate::auto_compaction::perform_auto_compaction(
                &entries,
                &config,
                None,
                Some(plan.reason),
                Some(sink),
                self.provider.as_ref(),
                writer,
            )) {
            Ok(Some(run)) => {
                // 压缩后**换用重建出来的上下文**（ref 压缩后用投影重建 `agent.state.messages`；
                // 不换的话 `get_state.messageCount` 会停在压缩前，S20 的 B 层第 16 行即此）。
                if let Ok(mut guard) = self.transcript.lock() {
                    *guard = run.messages.clone();
                }
                self.write_line(
                    &serde_json::json!({
                        "type": "compaction_end", "reason": plan.reason,
                        "result": run.result_event, "aborted": false, "willRetry": plan.will_retry,
                    })
                    .to_string(),
                );
            }
            Ok(None) => {}
            Err(error) => self.write_line(
                &serde_json::json!({
                    "type": "compaction_end", "reason": plan.reason, "aborted": false,
                    "willRetry": false, "errorMessage": error,
                })
                .to_string(),
            ),
        }
    }
}

/// 把循环事件实时投影并写到共享 stdout。
struct SharedSink(Arc<RpcShared>);

impl SharedSink {
    /// 投影并写出**一条**事件（不含原子段的开关）。
    fn write_event(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::QueueUpdate) {
            self.0.emit_queue_update();
            return;
        }
        // **直接写文本**：`to_json_event_line` 对 system 消息做了"原文拼接"（保键序），
        // 若再 `from_str` 成 `Value` 就会把顺序又排回去（这正是我第一版白改的原因）。
        self.0
            .write_line(&crate::print_mode::to_json_event_line(&event));
    }
}

impl EventSink for SharedSink {
    fn emit(&self, event: AgentEvent) {
        self.write_event(event);
        // 本线程的下一条事件写完 → 原子段结束（P4-15 步骤 1）；没开段时是空操作。
        self.0.section.end_if_owner();
    }

    fn emit_opening_section(&self, event: AgentEvent) {
        // 先开段、再写，写完后**不**关段：段持续到本线程的下一条事件（工具的首条进度上报）。
        self.0.section.begin();
        self.write_event(event);
    }

    fn end_section(&self) {
        self.0.section.end_if_owner();
    }
}

/// 主线程侧的入口。
pub struct RpcSession {
    shared: Arc<RpcShared>,
}

impl RpcSession {
    pub fn new(shared: Arc<RpcShared>) -> Self {
        Self { shared }
    }

    /// 处理一行命令；响应由本方法写出（`prompt` 先回响应，运行在后台继续）。
    pub fn handle(&self, line: &str) {
        let mut runtime = RuntimeBridge {
            shared: self.shared.clone(),
        };
        let response = handle_line(line, &mut runtime);
        // 键序是判据的响应（`data.__orderedRaw`）：**自己拼字符串**，因为 `Value` 的对象
        // 是 BTreeMap（按键名排序）。顶层字段照既有 `response_ok` 的实际输出顺序
        // （字母序：command, data, id, success, type）。
        let ordered = response
            .get("data")
            .and_then(|data| data.get(crate::rpc::ORDERED_RAW_KEY))
            .and_then(serde_json::Value::as_str);
        if let Some(raw) = ordered {
            let command = response
                .get("command")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let mut out = String::from("{\"command\":");
            out.push_str(&serde_json::Value::String(command.to_string()).to_string());
            out.push_str(",\"data\":");
            out.push_str(raw);
            if let Some(id) = response.get("id") {
                out.push_str(",\"id\":");
                out.push_str(&id.to_string());
            }
            out.push_str(",\"success\":true,\"type\":\"response\"}");
            self.shared.write_line(&out);
            return;
        }
        self.shared.write_line(&response.to_string());
    }
}

/// 把 `RpcRuntime` 的调用接到共享状态上。
struct RuntimeBridge {
    shared: Arc<RpcShared>,
}

/// 读磁盘上的会话文件为原始条目列表；尚未建会话时返回 None。
///
/// `get_tree`/`get_fork_messages` 都要它，所以抽出来共用。
fn session_entries_raw(shared: &RpcShared) -> Option<Vec<serde_json::Value>> {
    let path = match shared.session.lock() {
        Ok(guard) => guard.as_ref().map(|writer| writer.path().to_path_buf()),
        Err(_) => None,
    }?;
    let text = std::fs::read_to_string(&path).ok()?;
    let mut entries = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        entries.push(serde_json::from_str::<serde_json::Value>(line).ok()?);
    }
    Some(entries)
}

/// 会话投影里的消息（= TS 的 `agent.state.messages`：含 system、已应用投影规则）。
/// 读不出来时返回 `None`（调用方退回 transcript）。
fn state_messages(shared: &RpcShared) -> Option<Vec<Message>> {
    let path = shared
        .session
        .lock()
        .ok()?
        .as_ref()
        .map(|writer| writer.path().to_path_buf())?;
    let file = pi_core::session_io::SessionFile::read(&path).ok()?;
    let projection = pi_core::agent::projection::build_session_projection(
        &file.entries,
        pi_core::agent::projection::LeafSelector::Unspecified,
    )
    .ok()?;
    Some(projection.messages)
}

/// 会话文件里的条目**连同其原始行文本**（保序用）。
///
/// 与 [`session_entries_raw`] 的区别：那个只返回解析后的 `Value`（键序已丢），这个把
/// 原始行一并带出来，供 rpc 响应"按原文拼"（`get_entries`）。
fn session_entries_with_raw(shared: &RpcShared) -> Option<Vec<(serde_json::Value, String)>> {
    let path = shared
        .session
        .lock()
        .ok()?
        .as_ref()
        .map(|writer| writer.path().to_path_buf())?;
    let text = std::fs::read_to_string(&path).ok()?;
    let mut out = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
        out.push((value, line.to_string()));
    }
    Some(out)
}

/// 用户消息文本：`contentText(content, "")`（字符串原样；内容块只取 `text`，用 `""` 拼）。
fn message_text(entry: &serde_json::Value) -> String {
    match entry
        .get("message")
        .and_then(|message| message.get("content"))
    {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(serde_json::Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(_not_text) => String::new(),
    }
}

/// root→leaf 的条目路径（沿 `parentId` 上溯后反转）。`leaf` 为 `None` 时返回空
/// （TS 的 `buildSessionPath` 在 `leafId === null` 时返回 `[]`）。
fn branch_path(entries: &[serde_json::Value], leaf: Option<&str>) -> Vec<serde_json::Value> {
    let Some(leaf) = leaf else {
        return Vec::new();
    };
    let mut by_id: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();
    for entry in entries {
        if let Some(id) = entry.get("id").and_then(serde_json::Value::as_str) {
            by_id.insert(id.to_string(), entry.clone());
        }
    }
    let mut path = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut current = Some(leaf.to_string());
    while let Some(id) = current {
        if !seen.insert(id.clone()) {
            break;
        }
        let Some(entry) = by_id.get(&id) else {
            break;
        };
        path.push(entry.clone());
        current = entry
            .get("parentId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
    }
    path.reverse();
    path
}

/// 路径条目里的消息（**解析失败就报错**，不静默丢）。
fn path_messages(path: &[serde_json::Value]) -> Result<Vec<pi_ai::message::Message>, String> {
    let mut out = Vec::new();
    for entry in path {
        if entry.get("type").and_then(serde_json::Value::as_str) != Some("message") {
            continue;
        }
        let Some(message) = entry.get("message").cloned() else {
            continue;
        };
        let parsed = serde_json::from_value::<pi_ai::message::Message>(message)
            .map_err(|error| format!("Failed to read forked message: {error}"))?;
        out.push(parsed);
    }
    Ok(out)
}

impl RpcRuntime for RuntimeBridge {
    fn commands(&self) -> serde_json::Value {
        // ref `:683-710` 的字段与顺序：`{name, description, source, sourceInfo}`；
        // `sourceInfo` 形状照 golden/S24b：`{path, source:"auto", scope:"user", origin:"top-level", baseDir}`。
        // **扩展来源为空**：pi-rs 未实现扩展机制（已知限制）。
        let agent_dir = self.shared.agent_dir.to_string_lossy().to_string();
        let source_info = |path: &str| {
            serde_json::json!({
                "path": path,
                "source": "auto",
                "scope": "user",
                "origin": "top-level",
                "baseDir": agent_dir,
            })
        };
        let mut commands: Vec<serde_json::Value> = Vec::new();
        for template in &self.shared.prompt_templates {
            commands.push(serde_json::json!({
                "name": template.name,
                "description": template.description,
                "source": "prompt",
                "sourceInfo": source_info(&template.file_path),
            }));
        }
        for skill in &self.shared.skills {
            commands.push(serde_json::json!({
                "name": format!("skill:{}", skill.name),
                "description": skill.description,
                "source": "skill",
                "sourceInfo": source_info(&skill.file_path),
            }));
        }
        serde_json::json!({ "commands": commands })
    }

    fn prompt(&mut self, message: &str) -> Result<(), String> {
        // ref `agent-session.ts:1649/1841`：print 与 rpc **共用**同一 `/skill:` 展开路径；
        // 未命中 ⇒ 原样；读失败 ⇒ 按 ref `:1812-1819` 报错后原样放行。
        let message = match pi_core::skills::expand_skill_command(message, &self.shared.skills) {
            Ok(Some(expanded)) => expanded,
            Ok(None) => message.to_string(),
            Err((path, error)) => {
                eprintln!("Error: {error} ({path})");
                message.to_string()
            }
        };
        // ref `agent-session.ts:1646-1651`：**先 `/skill:`、后模板**，两者同一开关分支。
        let message = pi_core::prompt_templates::expand_prompt_template(
            &message,
            &self.shared.prompt_templates,
        );
        let message = message.as_str();
        if self.shared.busy.load(Ordering::SeqCst) {
            // **文案逐字照 TS**（`agent-session.ts:1655-1657`）：
            // "Agent is already processing. Specify streamingBehavior ('steer' or 'followUp') to queue the message."
            // 我此前自造了 "a run is already in progress"，被 golden/S16 的第 12 条指出。
            return Err(
                "Agent is already processing. Specify streamingBehavior ('steer' or 'followUp') to queue the message."
                    .to_string(),
            );
        }
        // **提示词前的压缩检查必须在写 ack 之前同步完成**（ref `agent-session.ts:1693-1698`：
        // 它在 `prompt()` 内、`agent.continue()` 之前；golden/S13g 的事件流第 1 行就是
        // `compaction_start`，而 `response(prompt)` 在其后）。此前我把这一步放在后台线程里
        // （`run_prompt` → `maybe_pre_prompt_compact`）⇒ 永远晚于 ack（S13g 的 B 层首差即此）。
        self.shared
            .maybe_pre_prompt_compact(&crate::auto_compaction::LineSink(std::sync::Arc::clone(
                &self.shared,
            )));
        // 后台线程跑；**响应先回**（`rpc-types.ts:116-121` 的异步语义），事件随后陆续到。
        let shared = self.shared.clone();
        let message = message.to_string();
        std::thread::spawn(move || shared.run_prompt(message));
        Ok(())
    }
    fn steer(&mut self, message: &str) -> Result<(), String> {
        self.shared
            .steering
            .push(pi_core::agent::user_message(message));
        // 入队后发 `queue_update`（ref `agent-session.ts:1881` 的 `_emitQueueUpdate()`）。
        // golden/S10 里它排在 `response{steer}` **之前**（第 13 条 vs 第 14 条）——
        // 因为 push 与发事件都在响应写出之前完成。
        self.shared.emit_queue_update();
        Ok(())
    }
    fn follow_up(&mut self, message: &str) -> Result<(), String> {
        self.shared
            .follow_up
            .push(pi_core::agent::user_message(message));
        // 同上（`agent-session.ts:1898`）。
        self.shared.emit_queue_update();
        Ok(())
    }
    fn set_thinking_level(&mut self, level: &str) -> Result<(), String> {
        // ref `rpc-mode.ts:499-502`：`session.setThinkingLevel(command.level)`。
        // 会话条目由 `session-manager.ts:1217` 写（golden/S16 的第 4 条 `thinking_level_change: high`）。
        if let Ok(mut guard) = self.shared.level.lock() {
            *guard = level.to_string();
        }
        // **会话级事件**：切换思考等级要发 `{"type":"thinking_level_changed","level":…}`
        // （ref `agent-session.ts:2269` 的 `this._emit({ type: "thinking_level_changed", level })`）。
        // golden/S16 的事件流第 1 条就是它，**排在该命令的 response 之前**。
        self.shared.write_line(&thinking_level_changed_json(level));
        self.shared.with_writer(|writer| {
            if let Err(error) = writer.append_thinking_level_change(level) {
                eprintln!("pi-rs: 写 thinking_level_change 失败：{error}");
            }
        });
        Ok(())
    }

    fn set_model(
        &mut self,
        provider: &str,
        model_id: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        // ref `rpc-mode.ts:472-480`：找不到模型时报 `Model not found: …`。
        // 本项目只配置了当前这一个 provider/model，按同样口径校验。
        validate_set_model(
            provider,
            model_id,
            &self.shared.provider_name,
            &self.shared.model_id,
            &self.shared.model,
        )?;
        // **即使模型没变也要写一条 `model_change`**（golden/S16 的第 5 条）：
        // `setModel` 是无条件写条目的。
        self.shared.with_writer(|writer| {
            if let Err(error) = writer.append_model_change(provider, model_id) {
                eprintln!("pi-rs: 写 model_change 失败：{error}");
            }
        });
        // 响应 `data` 是**完整模型对象**（`rpc-mode.ts:479` 的 `success(id, "set_model", model)`）。
        Ok(Some(self.shared.model_json.clone()))
    }

    fn abort(&mut self) -> Result<(), String> {
        if let Ok(guard) = self.shared.cancel.lock() {
            guard.cancel();
        }
        // **必须等这一轮真的结束**：ref `rpc-mode.ts:428-431` 是
        // `await session.abort(); return success(id, "abort")`——响应因此排在
        // 该轮所有事件**之后**（golden/S09a 的 `response{command:"abort"}` 是第 14 条，最后一条）。
        // 我此前立即返回，响应排到了 `message_end` 之前（第 10 条）。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while self.shared.busy.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(())
    }
    /// 会话树：形状 `{tree, leafId}`，节点 `{entry, children}`。
    ///
    /// 直接读**磁盘上的会话文件**（含本次运行刚写的条目），按 `parentId` 组树；
    /// 文件不存在（尚未建会话）时返回空树。〔session-manager.ts:1522；rpc-mode.ts:651〕
    fn tree(&self) -> Result<serde_json::Value, String> {
        let Some(entries) = session_entries_raw(&self.shared) else {
            return Ok(serde_json::json!({"tree": [], "leafId": null}));
        };
        let (tree, leaf_id) = pi_core::session_io::build_tree(&entries);
        // 键序取自 golden/S28 实测：`data` 是 `{"tree":…,"leafId":…}`。
        let leaf = match leaf_id {
            Some(id) => serde_json::Value::String(id),
            None => serde_json::Value::Null,
        };
        let tree_value = serde_json::Value::Array(tree);
        let ordered =
            pi_core::session_io::ordered_object(&[("tree", &tree_value), ("leafId", &leaf)]);
        Ok(serde_json::json!({crate::rpc::ORDERED_RAW_KEY: ordered}))
    }

    /// `fork`：默认 `position:"before"` —— 选中条目必须是**用户消息**（否则同样报
    /// `Invalid entry ID for forking`），分叉出的新会话只含 root→`parentId` 的路径，
    /// 并**切为当前会话**；返回值里的 `text` 是选中消息的文本。
    /// 〔`agent-session-runtime.ts:262-334`；路径逻辑同 `buildSessionPath`；rpc `:613-620`〕
    fn fork(&self, entry_id: &str, position: &str) -> Result<serde_json::Value, String> {
        let entries = session_entries_raw(&self.shared).unwrap_or_default();
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.get("id").and_then(serde_json::Value::as_str) == Some(entry_id))
        else {
            return Err("Invalid entry ID for forking".to_string());
        };
        // `position` 默认 "before"：那要求选中的是**用户消息**，分叉出它之前的历史；
        // `position: "at"`（`clone` 用）直接以该条目为界，不要求角色。
        let (target_leaf, selected_text) = if position == "at" {
            (Some(entry_id.to_string()), String::new())
        } else {
            let is_user_message = entry.get("type").and_then(serde_json::Value::as_str)
                == Some("message")
                && entry
                    .get("message")
                    .and_then(|message| message.get("role"))
                    .and_then(serde_json::Value::as_str)
                    == Some("user");
            if !is_user_message {
                return Err("Invalid entry ID for forking".to_string());
            }
            (
                entry
                    .get("parentId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                message_text(entry),
            )
        };
        let path = branch_path(&entries, target_leaf.as_deref());
        let messages = path_messages(&path)?;
        // 新会话文件：`<毫秒时间戳>_<新 id>.jsonl`。**数字开头**，因此排在既有
        // `session.jsonl` 之前——与 golden 的 A 层拼接顺序（run.mjs 按文件名 sort）一致。
        let dir = self
            .shared
            .session_dir
            .clone()
            .ok_or_else(|| "Failed to create forked session: no session dir".to_string())?;
        let previous = match self.shared.session.lock() {
            Ok(guard) => guard
                .as_ref()
                .map(|writer| writer.path().to_string_lossy().to_string()),
            Err(_) => None,
        };
        let new_id = uuid::Uuid::now_v7().to_string();
        let stamp = pi_core::timefmt::now_unix_millis();
        let file = dir.join(format!("{stamp}_{new_id}.jsonl"));
        let mut extra = serde_json::Map::new();
        if let Some(previous) = &previous {
            extra.insert(
                "parentSession".to_string(),
                serde_json::Value::String(previous.clone()),
            );
        }
        let header = pi_core::session::SessionHeader {
            kind: "session".to_string(),
            version: Some(pi_core::session::CURRENT_SESSION_VERSION),
            id: new_id,
            timestamp: pi_core::timefmt::unix_millis_to_iso8601(stamp),
            cwd: Some(self.shared.cwd.to_string_lossy().to_string()),
            extra,
        };
        let mut text = serde_json::to_string(&header)
            .map_err(|error| format!("Failed to serialize session header: {error}"))?;
        for item in &path {
            text.push('\n');
            text.push_str(&item.to_string());
        }
        std::fs::write(&file, text)
            .map_err(|error| format!("Failed to write forked session: {error}"))?;
        // 切为当前会话：写器、已写条数、上下文消息；新文件里没有 system，所以要重置标记。
        let mut writer = crate::session_writer::SessionWriter::open(&file)?;
        // 分叉出的会话也要补启动条目（与 `--continue` 启动时同样的规则：ref `sdk.ts:196`）。
        // golden/S28 实测：分叉后新会话是 `…, thinking_level_change, system, user, assistant`，
        // 且第二条 `get_tree` 的 `leafId` 就是它。
        // **只在路径里没有它时才补**（与 `--continue` 同规则，ref `sdk.ts:196`）：`clone` 常以
        // `thinking_level_change` 为叶子，复制过来的路径已含它，再补一条会多出一层
        // ——S29 的差分第 3 行抓到的就是这个。
        let has_thinking_level = path.iter().any(|entry| {
            entry.get("type").and_then(serde_json::Value::as_str) == Some("thinking_level_change")
        });
        if !has_thinking_level {
            let level = if self.shared.reasoning {
                "medium"
            } else {
                "off"
            };
            if let Err(error) = writer.append_thinking_level_change(level) {
                return Err(format!("Failed to write thinking_level_change: {error}"));
            }
        }
        if let Ok(mut guard) = self.shared.session.lock() {
            *guard = Some(writer);
        }
        if let Ok(mut guard) = self.shared.written.lock() {
            // **`written` 数的是 transcript（只含 message）**，不是路径条目数——
            // 路径里还有 `thinking_level_change` 这类非消息条目。设成 `path.len()` 会多算，
            // 于是运行结束时 `write_new_messages` 用 `skip(already)` 跳过新写的 assistant
            // （S29 的 A 层"少一条"就是这么来的）。
            *guard = messages.len();
        }
        if let Ok(mut guard) = self.shared.transcript.lock() {
            *guard = messages;
        }
        if let Ok(mut guard) = self.shared.system_written.lock() {
            *guard = false;
        }
        // 分叉出的会话是"已存在的会话"且里面没有 system → 本轮的 system 要 announce。
        self.shared
            .announce_system
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(serde_json::json!({"text": selected_text, "cancelled": false}))
    }

    /// `compact`：手动压缩（ref `agent-session.ts:2406-2550`）。
    ///
    /// P5-4a 覆盖 golden/S20 的**失败**路径：前一轮自动压缩已把最后一条变成 `compaction`
    /// ⇒ `prepareCompaction` 给不出方案 ⇒ ref `:2432-2436` 抛 `Already compacted` ⇒
    /// 事件 `compaction_start{reason:"manual"}` + `compaction_end{… errorMessage:"Compaction failed: Already compacted"}`，
    /// rpc 响应 `{success:false, error:"Already compacted"}`。
    ///
    /// **成功路径（发摘要请求、写条目、重建上下文）属 P5-4b**，这里明确报错、不假装成功。
    fn compact(&mut self, custom_instructions: Option<&str>) -> Result<serde_json::Value, String> {
        self.shared
            .write_line("{\"type\":\"compaction_start\",\"reason\":\"manual\"}");
        let entries = {
            // 锁中毒时按"读不到会话"处理（不 panic；禁令也要求非测试代码不得 expect）。
            let Ok(guard) = self.shared.session.lock() else {
                return Err("compact: session writer lock is poisoned".to_string());
            };
            let Some(writer) = guard.as_ref() else {
                return Err("compact: no session file".to_string());
            };
            let file = pi_core::session_io::SessionFile::read(writer.path())
                .map_err(|error| format!("Failed to read session: {error}"))?;
            file.entries.clone()
        };
        let settings = self.shared.settings;
        // ref `agent-session.ts:2429-2437`：先看能不能压；不能压要区分两种原因。
        let preparation = match pi_core::agent::compaction::prepare_compaction(&entries, &settings)
        {
            Ok(preparation) => preparation,
            Err(error) => {
                let message = error.to_string();
                self.shared
                    .write_line(&manual_compaction_end_line(&message));
                return Err(message);
            }
        };
        let Some(_preparation) = preparation else {
            let last_is_compaction = matches!(
                entries.last(),
                Some(pi_core::session::SessionEntry::Compaction { .. })
            );
            let message = if last_is_compaction {
                "Already compacted".to_string()
            } else {
                "Nothing to compact (session too small)".to_string()
            };
            self.shared
                .write_line(&manual_compaction_end_line(&message));
            return Err(message);
        };
        // 成功路径（ref `agent-session.ts:2406-2550`）：摘要请求**带自定义指令**（`:2429` 的
        // `compact(..., customInstructions, ...)`）→ 写 `compaction` 条目 → 用投影重建上下文。
        // 事件与 rpc 响应的形状取自 `golden/S20b`（TS 3 次稳定录制的基线）。
        let config = crate::auto_compaction::CompactionConfig {
            settings,
            context_window: self.shared.context_window,
            model: self.shared.model_id.clone(),
            model_provider: self.shared.provider_name.clone(),
            model_max_tokens: self.shared.max_tokens,
            model_reasoning: self.shared.reasoning,
            thinking_level: self.shared.level.lock().ok().map(|level| level.clone()),
        };
        let provider = std::sync::Arc::clone(&self.shared.provider);
        let runtime = std::sync::Arc::clone(&self.shared.runtime);
        let outcome = {
            let Ok(mut guard) = self.shared.session.lock() else {
                return Err("compact: session writer lock is poisoned".to_string());
            };
            let Some(writer) = guard.as_mut() else {
                return Err("compact: no session file".to_string());
            };
            runtime.block_on(crate::auto_compaction::perform_auto_compaction(
                &entries,
                &config,
                custom_instructions,
                Some("manual"),
                Some(&crate::auto_compaction::LineSink(std::sync::Arc::clone(
                    &self.shared,
                ))),
                provider.as_ref(),
                writer,
            ))
        };
        match outcome {
            Ok(Some(run)) => {
                // 压缩后换用重建出来的上下文（否则 `get_state.messageCount` 停在压缩前）。
                if let Ok(mut guard) = self.shared.transcript.lock() {
                    *guard = run.messages.clone();
                }
                self.shared.write_line(
                    &serde_json::json!({
                        "type": "compaction_end",
                        "reason": "manual",
                        "result": run.result_event,
                        "aborted": false,
                        "willRetry": false,
                    })
                    .to_string(),
                );
                Ok(run.result_event)
            }
            Ok(None) => {
                let message = "Nothing to compact (session too small)".to_string();
                self.shared
                    .write_line(&manual_compaction_end_line(&message));
                Err(message)
            }
            Err(error) => {
                self.shared.write_line(&manual_compaction_end_line(&error));
                Err(error)
            }
        }
    }

    /// `clear_queue`：清空两个队列并把**被清掉**的文本回给调用方，随后发一条 `queue_update`。
    /// 〔`rpc-mode.ts:433-435`；`agent-session.ts:2043-2051`〕
    fn clear_queue(&self) -> Result<serde_json::Value, String> {
        let steering = self.shared.steering.texts();
        let follow_up = self.shared.follow_up.texts();
        self.shared.steering.drain_all();
        self.shared.follow_up.drain_all();
        self.shared.emit_queue_update();
        Ok(serde_json::json!({"steering": steering, "followUp": follow_up}))
    }

    /// `switch_session`：打开**指定**会话文件并切过去。
    ///〔`rpc-mode.ts:605-611`；`agent-session-runtime.ts:203-217`〕
    fn switch_session(&self, session_path: &str) -> Result<serde_json::Value, String> {
        if session_path.is_empty() {
            return Err("Failed to switch session: no session path".to_string());
        }
        let path = std::path::PathBuf::from(session_path);
        let file = pi_core::session_io::SessionFile::read(&path)
            .map_err(|error| format!("Failed to open session: {error}"))?;
        let transcript = file
            .transcript()
            .map_err(|error| format!("Failed to read session: {error}"))?;
        let has_system = file.has_system_message();
        let has_thinking_level = file
            .entries
            .iter()
            .any(|entry| entry.kind() == "thinking_level_change");
        let mut writer = crate::session_writer::SessionWriter::open(&path)
            .map_err(|error| format!("Failed to open session file: {error}"))?;
        // 与 `--continue` 同规则：缺 `thinking_level_change` 才补（`sdk.ts:196`）。
        if !has_thinking_level {
            let level = if self.shared.reasoning {
                "medium"
            } else {
                "off"
            };
            if let Err(error) = writer.append_thinking_level_change(level) {
                return Err(format!("Failed to write thinking_level_change: {error}"));
            }
        }
        if let Ok(mut guard) = self.shared.session.lock() {
            *guard = Some(writer);
        }
        if let Ok(mut guard) = self.shared.transcript.lock() {
            *guard = transcript.clone();
        }
        if let Ok(mut guard) = self.shared.written.lock() {
            // **数的是消息数**（transcript），不是文件条目数——S29 的教训。
            *guard = transcript.len();
        }
        if let Ok(mut guard) = self.shared.system_written.lock() {
            *guard = has_system;
        }
        self.shared
            .announce_system
            .store(!has_system, std::sync::atomic::Ordering::Relaxed);
        Ok(serde_json::json!({"cancelled": false}))
    }

    /// `new_session`：在会话目录里**新建**一份会话并切过去——
    /// 复用正常启动时的建会话路径（`SessionWriter::create` 会写 header 与 `model_change`），
    /// 然后清空上下文与计数器（新会话是全新的）。
    /// 〔`rpc-mode.ts:437-444`；`agent-session-runtime.ts:227-230`〕
    fn new_session(&self) -> Result<serde_json::Value, String> {
        // **懒创建**：TS 的 `new_session` 并不会立刻在磁盘上建文件，文件是在**首次写消息**时出现的
        // （golden/S34 里只有种子那一份文件；对比过便知）。所以这里只把当前写器**清掉**，
        // 之后由既有的懒创建路径（`with_writer`/`write_new_messages`）在需要时建——
        // 这也正是 rpc 路径原本的行为（S16 分析里记过"命令期建会话时不写"）。
        if let Ok(mut guard) = self.shared.session.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.shared.transcript.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.shared.written.lock() {
            *guard = 0;
        }
        if let Ok(mut guard) = self.shared.system_written.lock() {
            *guard = false;
        }
        self.shared
            .announce_system
            .store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(serde_json::json!({"cancelled": false}))
    }

    /// `get_messages`：把已物化的 transcript 作为 `{messages}` 返回。
    /// 〔`rpc-mode.ts:674-676`〕
    fn messages(&self) -> Result<serde_json::Value, String> {
        // TS 的 `get_messages` 返回的是 `agent.state.messages`——**含 system 那条**，且是
        // **投影后的**上下文（`--continue` 载入或压缩之后都如此）。transcript 是循环输入
        // （不含 system），所以这里优先用会话投影，读不到时才退回 transcript。
        let transcript =
            state_messages(&self.shared).unwrap_or_else(|| match self.shared.transcript.lock() {
                Ok(guard) => guard.clone(),
                Err(_) => Vec::new(),
            });
        let items: Vec<serde_json::Value> = transcript
            .iter()
            .filter_map(|message| serde_json::to_value(message).ok())
            .collect();
        Ok(serde_json::json!({"messages": items}))
    }

    /// `get_last_assistant_text`：在**已物化**的消息里从后往前找第一条 assistant，
    /// 跳过 `stopReason == "aborted"` 且内容为空的；没有则 `text` 键消失。
    /// 〔`rpc-mode.ts:656-658`；`agent-session.ts:3972-3984`〕
    fn last_assistant_text(&self) -> Result<serde_json::Value, String> {
        let transcript = match self.shared.transcript.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        };
        let mut found: Option<String> = None;
        for message in transcript.iter().rev() {
            let value = match serde_json::to_value(message) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if value.get("role").and_then(serde_json::Value::as_str) != Some("assistant") {
                continue;
            }
            let empty = value
                .get("content")
                .and_then(|content| content.as_array())
                .map(Vec::is_empty)
                .unwrap_or(false);
            if value.get("stopReason").and_then(serde_json::Value::as_str) == Some("aborted")
                && empty
            {
                continue;
            }
            found = Some(message_text(&serde_json::json!({"message": value})));
            break;
        }
        Ok(match found {
            Some(text) => serde_json::json!({"text": text}),
            None => serde_json::json!({}),
        })
    }

    /// `get_entries`：全部条目（不含文件头），可选 `since` 只取其后的部分。
    /// 〔`rpc-mode.ts:638-648`；`session-manager.ts:1513-1515`〕
    fn entries(&self, since: Option<&str>) -> Result<serde_json::Value, String> {
        // **条目用会话行的原文拼进响应**：`serde_json::Map` 是 `BTreeMap`，解析后重建会把
        // 键序排成字母序（`details, firstKeptEntryId, fromHook, id, …`），而归一化工具按
        // **原文出现顺序**给 8 位短 id 编号 ⇒ `id` 与 `firstKeptEntryId` 的相对顺序一变，
        // 占位符编号就整体错位（golden/S20b、S20c 的差异即此）。与 `get_tree` 的
        // `ORDERED_RAW_KEY` 是同一套机制。
        let pairs = session_entries_with_raw(&self.shared).unwrap_or_default();
        let mut items: Vec<(serde_json::Value, String)> = pairs
            .into_iter()
            .filter(|(entry, _raw)| {
                entry.get("type").and_then(serde_json::Value::as_str) != Some("session")
            })
            .collect();
        if let Some(since) = since {
            let position = items.iter().position(|(entry, _raw)| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some(since)
            });
            let Some(position) = position else {
                return Err(format!("Entry not found: {since}"));
            };
            items = items.split_off(position + 1);
        }
        // `leafId` 是**当前会话的叶子**（最后一条条目），不是过滤结果。
        let leaf = session_entries_raw(&self.shared)
            .unwrap_or_default()
            .iter()
            .rev()
            .find_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
            .map(|id| serde_json::Value::String(id.to_string()).to_string())
            .unwrap_or_else(|| "null".to_string());
        let mut data = String::from("{\"entries\":[");
        for (index, (_entry, raw)) in items.iter().enumerate() {
            if index > 0 {
                data.push(',');
            }
            data.push_str(raw);
        }
        data.push_str("],\"leafId\":");
        data.push_str(&leaf);
        data.push('}');
        Ok(serde_json::json!({crate::rpc::ORDERED_RAW_KEY: data}))
    }

    /// `clone`：以当前叶子为界 fork（`position:"at"`）。
    fn clone_session(&self) -> Result<serde_json::Value, String> {
        let entries = session_entries_raw(&self.shared).unwrap_or_default();
        let leaf = entries
            .iter()
            .rev()
            .find_map(|entry| entry.get("id").and_then(serde_json::Value::as_str));
        let Some(leaf) = leaf else {
            return Err("Cannot clone session: no current entry selected".to_string());
        };
        // **`clone` 的响应只有 `cancelled`**（`rpc-mode.ts:627` 的 `success(id, "clone", { cancelled })`），
        // 不像 `fork` 还带 `text`——S29 的差分在这里抓到过一条多余的 `"text":""`。
        let mut result = self.fork(leaf, "at")?;
        if let Some(object) = result.as_object_mut() {
            object.remove("text");
        }
        Ok(result)
    }

    /// `get_fork_messages`：可 fork 的用户消息列表。
    fn fork_messages(&self) -> Result<serde_json::Value, String> {
        let entries = session_entries_raw(&self.shared).unwrap_or_default();
        Ok(serde_json::json!({"messages": pi_core::session_io::fork_messages(&entries)}))
    }

    fn state(&self) -> RpcSessionState {
        // `sessionFile`/`sessionId` 取自**已创建的会话写入器**（命令期可能已建）。
        // 写器已建 → 用真实文件；尚未建 → 用**启动时预生成**的值（TS 在文件写出前也会报）；
        // `--no-session` 两者都没有 → `null` / `""`。
        let (session_file, session_id) = match self.shared.session.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(writer) => (
                    Some(writer.path().to_string_lossy().to_string()),
                    writer.header_id().unwrap_or_default(),
                ),
                None => match &self.shared.precomputed_session {
                    Some((_iso, id, path)) => {
                        (Some(path.to_string_lossy().to_string()), id.clone())
                    }
                    None => (None, String::new()),
                },
            },
            Err(_) => (None, String::new()),
        };
        RpcSessionState {
            session_file,
            session_id,
            model: Some(self.shared.model_json.clone()),
            thinking_level: self
                .shared
                .level
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_else(|_| "off".to_string()),
            auto_compaction_enabled: true,
            is_streaming: self.shared.busy.load(Ordering::SeqCst),
            steering_mode: "one-at-a-time".to_string(),
            follow_up_mode: "one-at-a-time".to_string(),

            // TS 的 `agent.state.messages` **含 system 那条**（golden/S20b：一轮后 3、两轮后 5），
            // 而 transcript 是不含 system 的循环输入 ⇒ 计数从**会话投影**取（与 `get_messages`
            // 同源；投影失败时退回 transcript 长度，不猜）。
            message_count: rpc_message_count(&self.shared),
            pending_message_count: self.shared.steering.len() + self.shared.follow_up.len(),
        }
    }
}

/// `queue_update` 的线上文本（纯函数，便于单测：不碰 stdout、不碰锁）。
pub fn queue_update_json(steering: &[String], follow_up: &[String]) -> String {
    serde_json::json!({
        "type": "queue_update",
        "steering": steering,
        "followUp": follow_up,
    })
    .to_string()
}

/// `thinking_level_changed` 的线上文本（纯函数；ref `agent-session.ts:2269`）。
pub fn thinking_level_changed_json(level: &str) -> String {
    serde_json::json!({ "type": "thinking_level_changed", "level": level }).to_string()
}

/// `set_model` 的校验（纯函数，**含失败路径**）。
///
/// ref `rpc-mode.ts:472-480`：找不到模型时报 `Model not found: provider/id`。
/// 本项目只配置了当前这一个 provider/model，按同样口径校验。
pub fn validate_set_model(
    provider: &str,
    model_id: &str,
    current_provider: &str,
    current_model_id: &str,
    current_model: &str,
) -> Result<(), String> {
    if provider != current_provider || (model_id != current_model_id && model_id != current_model) {
        return Err(format!("Model not found: {provider}/{model_id}"));
    }
    Ok(())
}

/// 手动压缩失败时的 `compaction_end` 行（ref `agent-session.ts:2537-2549`：
/// `errorMessage = "Compaction failed: " + message`，`aborted` 为假、`willRetry` 为假、无 `result`）。
fn manual_compaction_end_line(message: &str) -> String {
    serde_json::json!({
        "type": "compaction_end",
        "reason": "manual",
        "aborted": false,
        "willRetry": false,
        "errorMessage": format!("Compaction failed: {message}"),
    })
    .to_string()
}

/// `get_state.messageCount`：会话投影里的消息条数（含 system），与 TS 的
/// `agent.state.messages.length` 对齐。投影读不出来时退回 transcript 长度。
fn rpc_message_count(shared: &RpcShared) -> usize {
    let path = match shared.session.lock() {
        Ok(guard) => guard.as_ref().map(|writer| writer.path().to_path_buf()),
        Err(_) => None,
    };
    if let Some(path) = path {
        if let Ok(file) = pi_core::session_io::SessionFile::read(&path) {
            if let Ok(projection) = pi_core::agent::projection::build_session_projection(
                &file.entries,
                pi_core::agent::projection::LeafSelector::Unspecified,
            ) {
                return projection.messages.len();
            }
        }
    }
    shared
        .transcript
        .lock()
        .map(|guard| guard.len())
        .unwrap_or_default()
}
