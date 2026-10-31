//! TUI 的终端 I/O 层（P7-3/W2）。
//!
//! 纯逻辑在 `pi_tui`（状态 + 事件 → 行文本）；本模块只做终端侧的事：
//! raw mode 的 RAII、按键读取、把 `pi_tui::render` 的行写到终端、以及**终端恢复**。
//!
//! 终端恢复覆盖四条路径（P7-3/W2 要求）：
//! 1. **正常退出**：`TerminalGuard::drop`；
//! 2. **panic**：`install_panic_hook()` 在 hook 里先恢复再交给原 hook；
//! 3. **SIGINT/SIGTERM**：`watch_signals()` 用 `tokio::signal::unix` 置位标志，主循环据此恢复并退出；
//! 4. 兜底：任何提前 return 都由 `TerminalGuard` 的 `Drop` 覆盖。
//!
//! 恢复动作（进/出成对，序列照 P7-3/W1 的实测：`\x1b[?2004h`/`\x1b[?25l` 进、`\x1b[?25h`/`\x1b[?2004l` 出）：
//! - 进入：`enable_raw_mode()`、隐藏光标、开启 bracketed paste；
//! - 退出：关闭 bracketed paste、显示光标、`disable_raw_mode()`。

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self};
use crossterm::{cursor, execute, terminal};
use pi_tui::TranscriptItem;
use pi_tui::{apply, TuiAction, TuiEvent, TuiState};

/// raw mode 的 RAII 守卫：`new()` 进入、`Drop` 恢复（幂等）。
pub struct TerminalGuard {
    restored: bool,
}

impl TerminalGuard {
    /// 进入 raw mode（含隐藏光标与 bracketed paste）。
    ///
    /// P7-6n：`r3` 走**主屏**（不进入备用屏幕，`tui-main-screen.ts` 的主屏模型），
    /// 因此按渲染器区分启动序列；R1/R2 的行为与之前逐字节一致。
    pub fn new() -> std::io::Result<Self> {
        // 经典 conhost（`cmd.exe` 窗口）默认**不开** VT 输出处理，而 crossterm 的
        // `enable_raw_mode()` 只改输入句柄（crossterm-0.28.1/src/terminal/sys/windows.rs:31-41）。
        // 真机实测（2026-10-29，`CreateProcess(CREATE_NEW_CONSOLE)` + pywinpty 对照）：
        // `GetConsoleMode(CONOUT$)=0x3` ⇒ 屏幕上是字面量 `[38;2;…m`，TUI 完全不可读。
        // `supports_ansi()` 会顺带给输出句柄置 `ENABLE_VIRTUAL_TERMINAL_PROCESSING`
        // （失败忽略：Windows Terminal / ConPTY 不需要，也不应因失败而不启动）。
        #[cfg(windows)]
        {
            let _ = crossterm::ansi_support::supports_ansi();
        }
        terminal::enable_raw_mode()?;
        let mut out = std::io::stdout();
        let _ = out.write_all(startup_bytes());
        let _ = out.flush();
        Ok(Self { restored: false })
    }

    /// 恢复终端（正常退出、panic hook、信号处理都走它）。
    pub fn restore(&mut self) {
        // 幂等闸门（P7-6n/W2）：正常退出 / Ctrl+C / 信号 / panic hook 共用，重复调用只写一次。
        if !take_exit_once(&mut self.restored) {
            return;
        }
        let mut out = std::io::stdout();
        // 顺序：先关 bracketed paste、再显示光标、最后离开备用屏幕并关 raw mode。
        // `r3` 没进过备用屏幕 ⇒ 不写 `LeaveAlternateScreen`（否则会把主屏内容清掉/切屏）。
        // r3 走 ref 的 stop 顺序（含 kitty pop）；R1/R2 保持原字节不变。
        let bytes = r3_exit_bytes(true, false);
        let _ = out.write_all(bytes.as_bytes());
        let _ = out.flush();
        // 退出前给输入线程一个**短宽限期**（30 ms），让它把终端对我们查询的应答
        // （kitty `\x1b[?...u`、DA1 `\x1b[...c`）读走。否则这些字节留在 tty 输入队列里，
        // 退出后会被 shell 读走 ⇒ 真机上出现 `bash: 1:3ua7: 未找到命令` 这类报错。
        std::thread::sleep(std::time::Duration::from_millis(30));
        let _ = terminal::disable_raw_mode();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// 把一条崩溃现场追加到 `path`（供 panic hook 调用；**不做任何可能再次 panic 的事**）。
///
/// 单独抽出来是为了能单测：`p78a_crash_log_tests` 断言文件被追加、历史内容不丢。
fn append_crash_entry(path: &std::path::Path, location: &str, message: &str) {
    let entry = format!("[panic] {location} :: {message}\n");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write as _;
        let _ = file.write_all(entry.as_bytes());
    }
}

/// 助手消息尾部的状态提示（照 ref `assistant-message.ts:182-199`）：
/// `error` → `Error: <errorMessage>`；`length` → 截断提示；`aborted` → 中止文案。
///
/// 实时渲染（`MessageEnd`）与 `-c`/`/resume` 重放共用同一份口径。
fn assistant_notice(
    stop_reason: &pi_ai::StopReason,
    error_message: Option<&str>,
) -> Option<String> {
    match stop_reason {
        pi_ai::StopReason::Error => Some(format!(
            "\nError: {}",
            error_message.unwrap_or("Unknown error")
        )),
        pi_ai::StopReason::Aborted => Some(format!(
            "\n{}",
            error_message
                .filter(|message| *message != "Request was aborted")
                .unwrap_or("Operation aborted")
        )),
        pi_ai::StopReason::Length => {
            Some("\nResponse was truncated before completion.".to_string())
        }
        other => {
            let _ = other;
            None
        }
    }
}

/// 本轮**新产出**的尾部：`produced` 是整段上下文，前 `history_size` 条是传进去的历史。
///
/// 只有尾部可以并回历史/写盘；把整段并回去会让历史每轮翻倍（真机现象：
/// "越往后轮次 Working 等得越久"，离线实测请求体 5.8k → 33MB、第 20 轮 21 秒）。
fn produced_tail(
    history_size: usize,
    produced: &[pi_ai::message::Message],
) -> &[pi_ai::message::Message] {
    produced
        .get(history_size.min(produced.len())..)
        .unwrap_or(&[])
}

/// 把会话里的历史消息转成 TUI transcript 项（`--continue` / `/resume` 重放用）。
///
/// ref 侧没有"只看裸字符串"的限制：`content` 既可能是字符串，也可能是块数组
/// （`[{"type":"text","text":"…"}]`），两种都要显示（`message.ts` 的 `UserMessage` 联合）。
fn transcript_from_messages(messages: &[pi_ai::message::Message]) -> Vec<pi_tui::TranscriptItem> {
    let mut out = Vec::new();
    for message in messages {
        match message {
            pi_ai::message::Message::User { content, .. } => {
                // `content` 两种形态都要重放：裸字符串（golden 的种子会话）与
                // **块数组**（真实会话文件里 user 消息一律是 `[{"type":"text",…}]`）。
                // 之前只处理裸字符串 ⇒ `-c` 续写只显示助手那一条（用户实测"`-c` 没啥用"）。
                let text = match content {
                    pi_ai::UserContent::Text(text) => text.clone(),
                    pi_ai::UserContent::Blocks(blocks) => text_of(blocks),
                };
                if !text.is_empty() {
                    out.push(pi_tui::TranscriptItem::User(text));
                }
            }
            pi_ai::message::Message::Assistant(assistant) => {
                // ref `interactive-mode.ts` 的 `renderInitialMessages` 复用同一个
                // `AssistantMessageComponent`：内容块**按顺序**渲染 —— thinking 块、工具调用行、
                // 文本块都要出现（真实会话里 assistant 常有 thinking + toolCall 而没有文本）；
                // 失败回合的 `content` 是空数组，只靠 `errorMessage` 表达，也要补一行 `Error: …`。
                let mut text = String::new();
                let mut out_items: Vec<pi_tui::TranscriptItem> = Vec::new();
                let push_text = |text: &mut String, items: &mut Vec<pi_tui::TranscriptItem>| {
                    if !text.is_empty() {
                        items.push(pi_tui::TranscriptItem::Assistant(std::mem::take(text)));
                    }
                };
                for block in &assistant.content {
                    match block {
                        pi_ai::types::Content::Text { text: chunk, .. } => text.push_str(chunk),
                        pi_ai::types::Content::Thinking { thinking, .. } => {
                            push_text(&mut text, &mut out_items);
                            if !thinking.is_empty() {
                                out_items.push(pi_tui::TranscriptItem::Thinking(thinking.clone()));
                            }
                        }
                        pi_ai::types::Content::ToolCall {
                            name, arguments, ..
                        } => {
                            push_text(&mut text, &mut out_items);
                            out_items.push(pi_tui::TranscriptItem::ToolCall {
                                id: format!("replay-{}", out_items.len()),
                                output: None,
                                is_error: false,
                                started_at: None,
                                took: None,
                                name: name.clone(),
                                summary: {
                                    let value = serde_json::Value::Object(arguments.clone());
                                    tool_summary(&value)
                                },
                            });
                        }
                        pi_ai::types::Content::Image { .. } => {}
                    }
                }
                if let Some(notice) =
                    assistant_notice(&assistant.stop_reason, assistant.error_message.as_deref())
                {
                    text.push_str(&notice);
                }
                push_text(&mut text, &mut out_items);
                out.extend(out_items);
            }
            pi_ai::message::Message::System { .. }
            | pi_ai::message::Message::ToolResult { .. }
            | pi_ai::message::Message::Custom { .. }
            | pi_ai::message::Message::BranchSummary { .. }
            | pi_ai::message::Message::CompactionSummary { .. } => {}
        }
    }
    out
}

/// 把内容块里的文本按行拼接（忽略 thinking / 工具调用 / 图片）。
fn text_of(blocks: &[pi_ai::types::Content]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            pi_ai::types::Content::Text { text, .. } => Some(text.as_str()),
            pi_ai::types::Content::Thinking { .. }
            | pi_ai::types::Content::ToolCall { .. }
            | pi_ai::types::Content::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 安装 panic hook：先把终端恢复，再调用原 hook（保证 panic 也能恢复）。
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut out = std::io::stdout();
        let _ = execute!(
            out,
            event::DisableBracketedPaste,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = out.flush();
        let _ = terminal::disable_raw_mode();
        // 崩溃现场落盘（ref 也写 `pi-tui-crash.log`，见 `tui-main-screen.ts:519`）：
        // 写到配置目录下的 `crash.log`，真机崩溃后可直接把该文件发回来定位。
        // 只做**追加**，不覆盖历史；写不了就算了（不能因为日志再 panic 一次）。
        if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
            let path = std::path::Path::new(&dir).join("crash.log");
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|text| (*text).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "（无消息）".to_string());
            let location = info
                .location()
                .map(|place| format!("{}:{}:{}", place.file(), place.line(), place.column()))
                .unwrap_or_else(|| "（无位置）".to_string());
            append_crash_entry(&path, &location, &message);
            let _ = writeln!(
                std::io::stderr(),
                "pi-rs 崩溃：{message}（{location}）——详情已写入 {}",
                path.display()
            );
        }
        previous(info);
    }));
}

/// 监听 SIGINT/SIGTERM，置位后由主循环负责恢复并退出。
///
/// Windows 没有 `SIGTERM`/进程组语义：Ctrl+C 由 crossterm 变成按键事件，
/// 主循环按 `TuiEvent::Quit` 处理，所以这里在 Windows 上是**空实现**。
#[cfg(windows)]
pub fn watch_signals(flag: Arc<AtomicBool>) {
    let _ = flag;
}

/// 监听 SIGINT/SIGTERM（Unix）。
#[cfg(unix)]
pub fn watch_signals(flag: Arc<AtomicBool>) {
    // **必须在独立线程的独立 runtime 里监听**：`run` 的主循环用阻塞式 `recv_timeout`，
    // 若信号任务挂在同一个 current-thread runtime 上就永不被 poll ⇒ 信号置不了位、
    // 进程不退出、终端停在 raw（P7-3/W4 实测：SIGINT/SIGTERM 两例卡住）。
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            let mut sigint =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()) {
                    Ok(stream) => stream,
                    Err(_) => return,
                };
            let mut sigterm =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(stream) => stream,
                    Err(_) => return,
                };
            tokio::select! {
                _ = sigint.recv() => flag.store(true, Ordering::SeqCst),
                _ = sigterm.recv() => flag.store(true, Ordering::SeqCst),
            }
        });
    });
}

/// 最小 base64 编码（RFC 4648，带 `=` 填充）——OSC 52 剪贴板需要它，避免新增依赖。
#[must_use]
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).map_or(0, |byte| u32::from(*byte));
        let b2 = chunk.get(2).map_or(0, |byte| u32::from(*byte));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[((triple >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((triple >> 12) & 0x3f) as usize]));
        out.push(if chunk.len() > 1 {
            char::from(TABLE[((triple >> 6) & 0x3f) as usize])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(TABLE[(triple & 0x3f) as usize])
        } else {
            '='
        });
    }
    out
}

/// 切换模型后刷新 footer 的 `x%/Nk`：`window` 跟着**当前**模型走（ref `footer.ts:151-158`
/// 每次渲染都取当前模型），百分比按同口径重算（有 usage 用 input，否则用 system 估算）。
fn refresh_model_window(
    state: &mut pi_tui::TuiState,
    parts: &AgentParts,
    history: &std::sync::Mutex<Vec<pi_ai::message::Message>>,
) {
    let window_tokens = context_window_of(parts);
    // 基数取**最后一条 assistant 的 usage**（ref `agent-session.ts:3858-3902` 的
    // `calculateContextTokens(usage)`），**不是** footer 上那个 `↑88M` 的累计值：
    // 真机上拿累计值一除就变成 `33416.0%/262k`。
    let last_input = history
        .lock()
        .ok()
        .and_then(|messages| {
            messages.iter().rev().find_map(|message| {
                if let pi_ai::message::Message::Assistant(assistant) = message {
                    let tokens = pi_ai::calculate_context_tokens(&assistant.usage);
                    (tokens > 0).then_some(tokens)
                } else {
                    None
                }
            })
        })
        .unwrap_or(0);
    let percent = if window_tokens == 0 {
        None
    } else if last_input > 0 {
        Some(last_input as f64 / window_tokens as f64 * 100.0)
    } else {
        let system_tokens = pi_core::agent::compaction::estimate_tokens(&parts.system_message);
        Some(system_tokens as f64 / window_tokens as f64 * 100.0)
    };
    let _ = apply(
        state,
        TuiEvent::Usage {
            input: state.usage_input,
            output: state.usage_output,
            percent,
            window: pi_tui::format_tokens(window_tokens),
            auto: true,
            cost: if state.usage_cost > 0.0 {
                Some(state.usage_cost)
            } else {
                None
            },
        },
    );
}

/// 切换模型时重建的结果：provider 实例 + 该模型的上下文窗口。
pub struct BuiltModel {
    pub provider: std::sync::Arc<dyn pi_ai::Provider>,
    pub context_window: u64,
}

/// 用 `models.json` + 内置目录重新解析并构造指定模型的 provider（`Ctrl+P` 用）。
fn rebuild_provider(
    parts: &AgentParts,
    provider: &str,
    model_id: &str,
) -> Result<BuiltModel, String> {
    let path = parts.agent_dir.join("models.json");
    // 没有 `models.json` 是正常状态（只放 `auth.json` 的机器）：按空配置走内置目录。
    let config = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<pi_ai::config::ModelsConfig>(&text)
            .map_err(|error| format!("models.json 解析失败：{error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            pi_ai::config::ModelsConfig::default()
        }
        Err(error) => return Err(format!("{} 读取失败：{error}", path.display())),
    };
    let catalog = crate::runtime::load_builtin_catalog(&parts.agent_dir);
    let resolved = config
        .resolve_with_catalog(provider, model_id, catalog.as_ref())
        .map_err(|error| format!("{provider}/{model_id} 解析失败：{error}"))?;
    let built = crate::runtime::build_provider(&resolved, None)?;
    Ok(BuiltModel {
        provider: built,
        context_window: resolved.context_window,
    })
}

/// 把 `/settings` 的一次改值落到界面状态上（ref 里由 `onChange` 回调改 settings 并重绘）。
///
/// 目前只有 `autocompact` 会影响可见输出：状态行的 ` (auto)` 后缀（`footer.ts:151`）。
fn apply_settings_change(
    state: &mut pi_tui::TuiState,
    change: Option<(&'static str, String)>,
    agent_dir: Option<&std::path::Path>,
) -> Option<String> {
    let (id, value) = change?;
    if id == "autocompact" {
        pi_tui::set_auto_compact(state, value == "true");
    }
    // 写回 settings.json（ref `onChange` → `SettingsManager.save()`）。
    // 数字项的界面值是**标签**（如 `5 min`）而不是毫秒时，`json_value_for` 会返回 `None`，
    // 此时不落盘（宁可少写也不写错值）；这类条目要等 c3 补标签→数值映射。
    if let Some(agent_dir) = agent_dir {
        // 界面值不是该类型的合法值（如 `http-idle-timeout` 的 `5 min` 标签）时
        // `persist_change` 返回 `Ok(false)` ⇒ 不写，避免写错值。
        let _ = crate::settings::persist_change(agent_dir, id, &value);
    }
    // ref 里这几项改值后会 `showStatus(...)` 往会话区追一行（`interactive-mode.ts:4837-4840`
    // `HTTP idle timeout:`、`:4841-4843` `Cache warming:`、`:4950-4953` `TUI mode:`）。
    match id {
        "http-idle-timeout" => Some(format!("HTTP idle timeout: {value}")),
        "cache-warming-mode" => Some(format!("Cache warming: {value}")),
        "tui-mode" => Some(format!("TUI mode: {value}")),
        "autocompact"
        | "auto-resize-images"
        | "block-images"
        | "skill-commands"
        | "show-hardware-cursor"
        | "editor-padding"
        | "output-padding"
        | "autocomplete-max-visible"
        | "clear-on-shrink"
        | "terminal-progress"
        | "steering-mode"
        | "follow-up-mode"
        | "transport"
        | "hide-thinking"
        | "mermaid-rendering"
        | "cache-miss-notices"
        | "collapse-changelog"
        | "quiet-startup"
        | "install-telemetry"
        | "default-project-trust"
        | "double-escape-action"
        | "tree-filter-mode"
        | "warnings"
        | "model-thinking"
        | "fullscreen-exit-output"
        | "fullscreen-scrollbar"
        | "fullscreen-copy-on-select"
        | "theme" => None,
        other => {
            // 未在表里的 id（新增条目忘记登记）不产生状态行，不 panic。
            let _ = other;
            None
        }
    }
}

/// 当前生效的模型与它的 provider（P7-7f M：`Ctrl+P` / `/model` 选中会替换）。
pub struct CurrentModelState {
    /// 模型 id（`models.json` 的 `id`）。
    pub model_id: String,
    /// provider 标识（`openai-completions` 等）。
    pub provider_name: String,
    /// 已构造的 provider 实例。
    pub provider: std::sync::Arc<dyn pi_ai::Provider>,
    /// 该模型的上下文窗口 token 数（切换模型时跟着换；footer 的 `x%/Nk` 与
    /// 无 usage 时的百分比估算都读它，ref `footer.ts:151-158` 用的是**当前**模型）。
    pub context_window: u64,
}

/// 当前生效模型的上下文窗口：优先取 `model_state`（切换后已更新），
/// 退化到装配时的值（`context_window` 与 `model_state` 不一致只可能发生在测试桩里）。
pub fn context_window_of(parts: &AgentParts) -> u64 {
    parts
        .model_state
        .lock()
        .map(|state| state.context_window)
        .ok()
        .filter(|window| *window > 0)
        .unwrap_or(parts.context_window)
}

/// 打开中的选择器（P7-7）。两者共用一套按键拦截：字符进搜索框、Enter/Esc 关闭。
enum SelectorUi {
    /// `/model`（P7-7a）。
    Model(crate::model_selector::ModelSelector),
    /// `/thinking`（P7-7b）。
    Thinking(crate::thinking_selector::ThinkingSelector),
    /// `/settings`（P7-7c1）；第二项是面板宽度（行内两列对齐用）。
    Settings(crate::settings_selector::SettingsSelector, usize),
    /// `/settings` → Theme 子菜单（P7-7c3b）。
    Theme(crate::settings_selector::ThemeSubmenu),
    /// `/settings` → Warnings 子菜单（P7-7c3c）。
    Warnings(crate::settings_selector::WarningsSubmenu),
    /// `/settings` → `Default thinking level per model` 两步子菜单（P7-7c3d）。
    ModelThinking(crate::settings_selector::ModelThinkingSubmenu),
    /// `/resume` 会话选择器（P7-7d1）；第二项是面板宽度（头部右对齐用）。
    Resume(crate::settings_selector::ResumeSelector, usize),
    /// `/tree` 会话树（P7-7e1/e2）；第二项是面板宽度（头部边框用）。
    Tree(crate::settings_selector::TreeSelector, usize),
    /// `/tree` 选中非叶子后的 `Summarize branch?` 提示（P7-7e e4a）。
    Summarize(crate::settings_selector::SummarizeSelector),
    /// `/tree` 的 `shift+l` 标签输入（P7-7e）；第二项是面板宽度（头部块用）。
    Label(crate::settings_selector::LabelInput, usize),
}

impl SelectorUi {
    fn push(&mut self, ch: char) {
        match self {
            SelectorUi::Model(sel) => sel.push(ch),
            SelectorUi::Thinking(sel) => sel.push(ch),
            SelectorUi::Settings(sel, _) => sel.push(ch),
            // 子菜单没有搜索框（`SelectSubmenu` 未开 searchable）⇒ 字符忽略。
            SelectorUi::Theme(_) | SelectorUi::Warnings(_) => {}
            SelectorUi::ModelThinking(sel) => sel.push(ch),
            SelectorUi::Resume(_, _) => {}
            SelectorUi::Tree(_, _) | SelectorUi::Summarize(_) => {}
            SelectorUi::Label(sel, _) => sel.push(ch),
        }
    }

    fn pop(&mut self) {
        match self {
            SelectorUi::Model(sel) => sel.pop(),
            SelectorUi::Thinking(sel) => sel.pop(),
            SelectorUi::Settings(sel, _) => sel.pop(),
            SelectorUi::Theme(_) | SelectorUi::Warnings(_) => {}
            SelectorUi::ModelThinking(sel) => sel.pop(),
            SelectorUi::Resume(_, _) => {}
            SelectorUi::Tree(_, _) | SelectorUi::Summarize(_) => {}
            SelectorUi::Label(sel, _) => sel.pop(),
        }
    }

    /// 上移选中项（只有 `/settings` 支持；其余选择器单条/忽略）。
    fn move_up(&mut self) {
        match self {
            SelectorUi::Settings(sel, _) => sel.move_up(),
            SelectorUi::Theme(sel) => sel.move_up(),
            SelectorUi::Warnings(_) => {}
            SelectorUi::ModelThinking(sel) => sel.move_up(),
            SelectorUi::Resume(sel, _) => sel.move_up(),
            SelectorUi::Tree(sel, _) => sel.move_up(),
            SelectorUi::Summarize(sel) => sel.move_up(),
            SelectorUi::Label(_, _) => {}
            SelectorUi::Model(sel) => sel.move_up(),
            SelectorUi::Thinking(_) => {}
        }
    }

    /// 下移选中项（只有 `/settings` 支持）。
    fn move_down(&mut self) {
        match self {
            SelectorUi::Settings(sel, _) => sel.move_down(),
            SelectorUi::Theme(sel) => sel.move_down(),
            SelectorUi::Warnings(_) => {}
            SelectorUi::ModelThinking(sel) => sel.move_down(),
            SelectorUi::Resume(sel, _) => sel.move_down(),
            SelectorUi::Tree(sel, _) => sel.move_down(),
            SelectorUi::Summarize(sel) => sel.move_down(),
            SelectorUi::Label(_, _) => {}
            SelectorUi::Model(sel) => sel.move_down(),
            SelectorUi::Thinking(_) => {}
        }
    }

    /// 是否为子菜单（`Esc`/`Enter` 语义分叉用）。
    fn is_submenu(&self) -> bool {
        matches!(
            self,
            SelectorUi::Theme(_) | SelectorUi::Warnings(_) | SelectorUi::ModelThinking(_)
        )
    }

    /// 是否为 `/settings` 选择器（`Enter` 语义分叉用）。
    fn is_settings(&self) -> bool {
        matches!(self, SelectorUi::Settings(_, _))
    }

    /// 激活当前条目（只有 `/settings` 支持：循环改值）；返回被改动的 `(id, 新值)`。
    fn activate(&mut self) -> Option<(&'static str, String)> {
        match self {
            SelectorUi::Settings(sel, _) => sel.activate(),
            // 子菜单的 Enter 由调用点单独处理（要回退到父列表）。
            SelectorUi::Theme(_)
            | SelectorUi::Warnings(_)
            | SelectorUi::ModelThinking(_)
            | SelectorUi::Resume(_, _)
            | SelectorUi::Tree(_, _)
            | SelectorUi::Summarize(_)
            | SelectorUi::Label(_, _)
            | SelectorUi::Model(_)
            | SelectorUi::Thinking(_) => None,
        }
    }

    fn lines(&self) -> Vec<String> {
        match self {
            SelectorUi::Model(sel) => sel.lines(),
            SelectorUi::Thinking(sel) => sel.lines(),
            SelectorUi::Settings(sel, cols) => sel.lines(*cols),
            SelectorUi::Theme(sel) => sel.lines(),
            SelectorUi::Warnings(sel) => sel.lines(),
            SelectorUi::ModelThinking(sel) => sel.lines(),
            SelectorUi::Resume(sel, cols) => sel.lines(*cols),
            SelectorUi::Tree(sel, cols) => sel.lines(*cols),
            SelectorUi::Summarize(sel) => sel.lines(),
            SelectorUi::Label(sel, cols) => sel.lines(*cols),
        }
    }
}

/// 一个"可切换模型"：`provider + modelId`。
///
/// ref 的 `settings.enabledModels` 形如 `provider/modelId`，而模型 id 自身可能含 `/`
/// （如 `deepseek/deepseek-v4.1-flash`）⇒ 解析时按**第一个** `/` 切分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub provider: String,
    pub id: String,
}

/// `Ctrl+P` 的下一个候选（ref `interactive-mode.ts:4378-4394` 的循环语义）。
///
/// 真机实测：`enabledModels` 里只要有一条解析不出来（如 models.json 里没有该 id），
/// 旧实现会把**同一条错误重复到永远**——当前模型不变 ⇒ 下一次又算到同一个 `index+1`，
/// 其余模型统统切不到。这里改成逐个往后试，只返回**第一个能解析**的模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextModel<P> {
    /// 找到可用的下一个模型（带回已构建好的 provider，避免重复构建）。
    Chosen(ModelChoice, P),
    /// 可切换列表不足 2 项（ref 显示 `Only one model available`）。
    OnlyOne,
    /// 列表里的其它项全部解析失败（带回最后一条错误文案）。
    NoneSwitchable(String),
}

/// 从 `current` 往后逐个尝试，跳过解析失败的项。
///
/// `try_build` 负责把候选模型变成 provider（真实实现是 [`rebuild_provider`]，
/// 单测里传闭包以便构造失败场景）。
fn next_switchable_model<P>(
    choices: &[ModelChoice],
    current_id: &str,
    current_provider: &str,
    mut try_build: impl FnMut(&ModelChoice) -> Result<P, String>,
) -> NextModel<P> {
    if choices.len() <= 1 {
        return NextModel::OnlyOne;
    }
    let start = choices
        .iter()
        .position(|choice| choice.id == current_id && choice.provider == current_provider)
        .unwrap_or(0);
    let mut last_error: Option<String> = None;
    for offset in 1..choices.len() {
        let choice = &choices[(start + offset) % choices.len()];
        match try_build(choice) {
            Ok(provider) => return NextModel::Chosen(choice.clone(), provider),
            Err(error) => last_error = Some(error),
        }
    }
    NextModel::NoneSwitchable(last_error.unwrap_or_else(|| "没有可切换的模型".to_string()))
}

/// 起一轮 agent run 所需的部件（W3-2：由 `main.rs` 用与 print 模式相同的装配构造后传入）。
pub struct AgentParts {
    /// 已构造好的 provider（print 路径同一个函数产物）。
    pub provider: std::sync::Arc<dyn pi_ai::Provider>,
    /// 已解析的工具集（`print_mode::default_tools` 等）。
    pub tools: Vec<std::sync::Arc<dyn pi_core::tools::tool::Tool>>,
    /// 循环配置（模型/系统提示/上限等）。
    pub config: pi_core::agent::LoopConfig,
    /// 上下文窗口 token 数（footer 的 `x%/Nk` 用它换算）。
    pub context_window: u64,
    /// 会话里 system 条目的**原始形态**（`content: ""` + `sections` + `toolsAdded`）：footer 百分比
    /// 的估算要经 `estimate_tokens` 走 system 分支，照 ref `compaction.ts:320-336`。
    pub system_message: pi_ai::message::Message,
    /// 工作目录（`/resume` 的 `Current Folder` 作用域按它过滤会话）。
    pub cwd: String,
    /// `--continue/-c` 时被续写的会话文件路径（`None` = 新建会话）。
    pub resumed_path: Option<std::path::PathBuf>,
    /// `--continue` 时从会话里读出的历史消息（按顺序进 `history` 与 transcript）。
    pub initial_messages: Vec<pi_ai::message::Message>,
    /// 该 provider 下可切换的模型 id 列表（`models.json` 顺序；Ctrl+P 循环用）。
    pub available_models: Vec<ModelChoice>,
    /// `settings.enabledModels` **是否显式配置**（ref 的 `scopedModels` 非空才是）。
    /// 没有配置时 `available_models` 会回落到 provider 下的全部模型，但那**不是** scope，
    /// 启动屏不应打印 `Model scope:` 行（T01 实测：多打一行会让整屏下移）。
    pub scoped_models_configured: bool,
    /// `models.json` 里**配置了模型的 provider 名**（去重）。
    /// ref ``footerData.getAvailableProviderCount()``（`components/footer.ts:193-200`）用它决定
    /// 是否显示 `(provider)` 前缀——不是"当前 scoped 列表里的 provider"。
    pub provider_names: Vec<String>,
    /// `/login` 的 provider 列表（`models.json` 的 provider 键；ref 遍历运行时 provider）。
    pub login_providers: Vec<String>,
    /// `-c`/`/resume` 时从会话里恢复的思考档位（最后一个 `thinking_level_change`）。
    pub initial_thinking_level: Option<String>,
    /// 当前模型是否推理（`models.json` 的 `reasoning`；决定思考档位集合）。
    pub supports_thinking: bool,
    /// 当前模型状态：模型 id + 已构造的 provider（Ctrl+P / `/model` 选中会替换它）。
    pub model_state: std::sync::Arc<std::sync::Mutex<CurrentModelState>>,
    /// 会话目录（`PI_CODING_AGENT_SESSION_DIR`）：`/resume` 扫描 `*.jsonl`。
    pub session_dir: std::path::PathBuf,
    /// 配置目录（`PI_CODING_AGENT_DIR`）：`/settings` 改值要写回
    /// `<agent_dir>/settings.json`（ref `settings-manager.ts:668-700`）。
    pub agent_dir: std::path::PathBuf,
    /// 会话写入器（`--no-session` 时为 `None`）：每轮结束把新消息按既有格式落盘（W4 会话层对拍的前提）。
    pub writer: Option<std::sync::Arc<std::sync::Mutex<crate::session_writer::SessionWriter>>>,
}

/// 启动字节（切默认后只剩主屏路径）：`2004h` → kitty push/查询/DA1 → `2031h` → DSR 996 → 隐藏光标。
#[must_use]
pub fn startup_bytes() -> &'static [u8] {
    // Windows 走 crossterm 事件路径，**没有** Unix 那套协商层（`input.rs` + `term_negotiate`
    // + `osc11`）来消费查询应答；而 conhost 会把 DA1 应答（`\x1b[?1;0c`）写回输入缓冲，
    // 于是这些字符被当成按键插进编辑器（真机实测：启动后编辑框里已有 `[?1;0c`，`Ctrl+D`
    // 与 `Ctrl+L` 因 `input` 非空而失效）。不发查询就不会有应答；kitty 的 CSI-u 编码同样
    // 没有解析方，因此在 Windows 上也不发 kitty 查询/推送。
    #[cfg(windows)]
    {
        // **不在 Windows 上请求括号粘贴**（P8e-17）：ConPTY 会把 wezterm 回送的
        // `ESC[200~`/`ESC[201~` 拆坏——真机事件日志（`PI_TUI_EVENT_DUMP`）里整段粘贴
        // 只有按键、**一个 Esc 都没有**，末尾却多出 `1~`：说明 `ESC[20` 被 ConPTY 在
        // 我们之下吞掉、`1~` 当成普通字符送进来，应用侧无法阻止。
        // 不请求标记时终端只发纯文本，交给 `PasteBurst` 合批（conhost 通道本来就这么走）。
        b"\x1b[?2031h\x1b[?996n\x1b[?25l"
    }
    #[cfg(not(windows))]
    {
        b"\x1b[?2004h\x1b[>7u\x1b[?u\x1b[c\x1b[?2031h\x1b[?996n\x1b[?25l"
    }
}

/// r3 的退出字节：`2004l` → `\x1b[<u`（kitty pop）→（若开过）`\x1b[>4;0m` → `\x1b[?25h`。
///
/// 顺序依据 `terminal.ts:428` / `:434-440` / `:441`。`modify_other_keys` 由协商层状态决定
/// （本批未接线 ⇒ 恒为 `false`，但仍按参数保留以便接线后直接使用）。
#[must_use]
pub fn r3_exit_bytes(kitty_pushed: bool, modify_other_keys: bool) -> String {
    let mut out = String::from("\x1b[?2004l");
    if kitty_pushed {
        out.push_str("\x1b[<u");
    }
    if modify_other_keys {
        out.push_str("\x1b[>4;0m");
    }
    out.push_str("\u{1b}[?2031l");
    out.push_str("\x1b[?25h");
    out
}

/// 退出前把光标移到**内容末尾下方**并换行（`tui-main-screen.ts:168-176` `beforeTerminalStop`）。
///
/// TS：`write(" ")` → 按 `targetRow - hardwareCursorRow` 写 `\e[nA/B` → `"\r\n"`。
/// 该序列会让主屏**多滚一行**，因此 golden 的全文比"仅内容"多一行（P7-6o(续3)/W2.4 实测）。
/// r3 未实现它时，T05 的全文会比 TS 少一行。返回要写出的字节。
#[must_use]
pub fn before_terminal_stop_bytes(target_row: usize, hardware_cursor_row: usize) -> String {
    let mut out = String::from(" ");
    let delta = target_row as i64 - hardware_cursor_row as i64;
    if delta > 0 {
        out.push_str(&format!("\x1b[{delta}B"));
    } else if delta < 0 {
        out.push_str(&format!("\x1b[{}A", -delta));
    }
    out.push_str("\r\n");
    out
}

/// 退出写的**幂等闸门**（P7-6n/W2）：第一次返回 `true` 并置位，之后恒 `false`。
pub fn take_exit_once(restored: &mut bool) -> bool {
    if *restored {
        return false;
    }
    *restored = true;
    true
}

/// 把请求的思考档位夹到**模型支持的档位集合**内（ref `sdk.ts:251-254`
/// 的 `clampThinkingLevel`）：不支持的档位取"不超过它的最高支持档"，
/// 比所有支持档都低时取最低支持档。纯函数，便于单测。
fn clamp_thinking_level(available: &[String], requested: &str) -> String {
    const ORDER: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    if available.iter().any(|level| level == requested) {
        return requested.to_string();
    }
    let Some(requested_index) = ORDER.iter().position(|item| *item == requested) else {
        // 未知档位：ref `models.ts:942-943` 返回 `availableLevels[0] ?? "off"`。
        return available
            .first()
            .cloned()
            .unwrap_or_else(|| "off".to_string());
    };
    // ref `models.ts:945-952`：**先向上找**（requestedIndex → 更大），再向下回退。
    for index in requested_index..ORDER.len() {
        if let Some(candidate) = ORDER.get(index) {
            if available.iter().any(|level| level == candidate) {
                return (*candidate).to_string();
            }
        }
    }
    for index in (0..requested_index).rev() {
        if let Some(candidate) = ORDER.get(index) {
            if available.iter().any(|level| level == candidate) {
                return (*candidate).to_string();
            }
        }
    }
    available
        .first()
        .cloned()
        .unwrap_or_else(|| "off".to_string())
}

/// 登录成功后解析该 provider 的默认模型（ref `completeProviderAuthentication`：
/// 刷新可用模型并选中默认模型）。取 `settings.defaultModel`（若属于该 provider），
/// 否则取 `models.json` 里该 provider 的第一个模型；都没有则保持 `no-model`。
fn model_after_login(agent_dir: &std::path::Path, provider: &str) -> Option<String> {
    // 文件可能根本不存在（零配置机器）：任一缺失都不算错误，继续往下回落内置目录。
    let settings: serde_json::Value = std::fs::read_to_string(agent_dir.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    let models: serde_json::Value = std::fs::read_to_string(agent_dir.join("models.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    let ids: Vec<String> = models
        .get("providers")
        .and_then(|providers| providers.get(provider))
        .and_then(|entry| entry.get("models"))
        .and_then(|models| models.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|model| model.get("id")?.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let default_id = settings
        .get("defaultModel")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let default_provider = settings
        .get("defaultProvider")
        .and_then(|value| value.as_str());
    if default_provider == Some(provider) {
        if let Some(id) = default_id.filter(|id| ids.iter().any(|known| known == id)) {
            return Some(id);
        }
    }
    if let Some(id) = ids.first() {
        return Some(id.clone());
    }
    // `models.json` 没配这个 provider 时，用内置目录里该 provider 的第一个模型
    crate::runtime::load_builtin_catalog(agent_dir)
        .and_then(|catalog| catalog.first_model(provider).map(str::to_string))
}

/// `/login` 面板条目（ref `interactive-mode.ts:354-402` 的选择器条目）。
#[derive(Debug, Clone)]
pub struct LoginItem {
    pub label: String,
    /// `None` = 认证方式这一层（"Sign in with an account" / "Sign in with an API key"）
    pub id: Option<String>,
    pub auth_type: crate::login_options::AuthType,
}

/// `/login` 面板渲染（选中项前缀 `→`，其余缩进；提示行照 ref 原文）。
fn login_render(state: &mut pi_tui::TuiState, items: &[LoginItem], index: usize) {
    let mut lines = vec![" Select authentication method:".to_string(), String::new()];
    if items.iter().all(|item| item.id.is_some()) {
        // provider 这一层：ref 的标题是同一条 "Select authentication method:" 之后的列表，
        // 这里保持同样形式的列表，不额外加标题。
        lines.truncate(1);
        lines.push(String::new());
    }
    // ref `oauth-selector.ts:115-150`：最多 8 项，窗口以选中项为中心，
    // 被裁剪时补一行 `  (n/total)`。此前把全部 provider 一次铺出来，条目多于屏幕高度
    // 时视口会滚到底部 —— 选中的那项被顶出视图（用户报的"焦点停在最后一个供应商"）。
    let (start, end) = pi_tui::selector_window(items.len(), index, 8);
    for (position, item) in items.iter().enumerate().take(end).skip(start) {
        let marker = if position == index { " → " } else { "   " };
        lines.push(format!("{marker}{}", item.label));
    }
    if start > 0 || end < items.len() {
        lines.push(pi_tui::selector_scroll_hint(index, items.len()));
    }
    lines.push(String::new());
    lines.push(" ↑↓ navigate  enter select  escape/ctrl+c cancel".to_string());
    state.open_panel(pi_tui::PanelPlacement::BelowEditor, lines);
}

/// 认证方式层（ref `/login` 第一屏）。
fn login_method_items() -> Vec<LoginItem> {
    vec![
        LoginItem {
            label: "Sign in with an account".to_string(),
            id: None,
            auth_type: crate::login_options::AuthType::Oauth,
        },
        LoginItem {
            label: "Sign in with an API key".to_string(),
            id: None,
            auth_type: crate::login_options::AuthType::ApiKey,
        },
    ]
}

/// provider 层（按认证方式过滤）。
fn login_provider_items(
    providers: &[String],
    auth_type: crate::login_options::AuthType,
) -> Vec<LoginItem> {
    crate::login_options::login_options(providers)
        .into_iter()
        .filter(|option| option.auth_type == auth_type)
        .map(|option| LoginItem {
            label: format!("{} ({})", option.name, option.auth_type.as_str()),
            id: Some(option.id),
            auth_type: option.auth_type,
        })
        .collect()
}

/// 登录成功后重建 agent 部件的回调（无模型启动时 `parts` 为 None，登录后要用新凭据建出来）。
pub type RebuildParts = Box<dyn Fn() -> Option<(std::sync::Arc<AgentParts>, String)> + Send + Sync>;

pub fn run(
    mut state: TuiState,
    mut parts: Option<std::sync::Arc<AgentParts>>,
    rebuild: Option<RebuildParts>,
) -> std::io::Result<i32> {
    install_panic_hook();
    let mut guard = TerminalGuard::new()?;
    let sig_flag = Arc::new(AtomicBool::new(false));
    watch_signals(Arc::clone(&sig_flag));

    // 输入读取：crossterm `event::read` 的**独立线程 + mpsc**（W0(a) 的 A/B 对照结论：
    // B 在 T02/T03、UTF-8 多字节、方向键四类用例下均正常 ⇒ 采用 B；此前'crossterm 收不到输入'
    // 的结论是 helper 写块被吞掉造成的假象，已在 notes/decisions.md 记更正）。
    let (tx, rx) = std::sync::mpsc::channel::<TuiEvent>();
    // 跨 Submit 存活的消息历史：第二轮请求体因此天然包含第一轮的 user+assistant（T08 的重点）。
    // 回合串行化状态（P7-7f/perf）：`busy` = 有回合在跑；`queued_inputs` = 等待提交的输入。
    let mut busy = false;
    // `/login` 无参面板状态：(候选项, 选中下标)
    let mut login_panel: Option<(Vec<LoginItem>, usize)> = None;
    let mut queued_inputs: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    // `--continue` 时把历史消息灌进 `history`，并把可显示的部分放进 transcript
    // （否则续写的会话在 TUI 里是空屏、`/tree` 也看不到历史）。
    // 思考档位集合（ref `ai/src/models.ts:924-933`）：非推理模型只有 `off`，
    // 推理模型是 `EXTENDED_THINKING_LEVELS`（`off/minimal/low/medium/high/xhigh/max`）。
    // 状态区右侧与初始状态行：ref `footer.ts:170-171` 取**已解析模型**的 id、`:193-200` 在可用
    // provider 多于 1 个时加 `(provider)` 前缀、`:151-158` 的初始上下文百分比按该模型的
    // `contextWindow` 算。此前右侧取 settings 的 `defaultModel`、状态行写死 `0.0%/128k`
    // （真机实测 200k 模型启动时显示 `0.0%/128k (auto)`，与 ref 差 72k）。
    if let Some(parts_ref) = parts.as_ref() {
        if let Ok(current) = parts_ref.model_state.lock() {
            state.model = current.model_id.clone();
            state.model_provider = current.provider_name.clone();
        }
        // 优先用"models.json 里配了模型的 provider 数"（与 ref 的 getAvailableProviderCount 同口径）；
        // 没有该信息时退回 scoped 列表里的 provider 去重数。
        state.login_providers = parts_ref.login_providers.clone();
        state.available_provider_count = if parts_ref.provider_names.is_empty() {
            let mut providers: Vec<&str> = parts_ref
                .available_models
                .iter()
                .map(|choice| choice.provider.as_str())
                .collect();
            providers.sort_unstable();
            providers.dedup();
            providers.len()
        } else {
            parts_ref.provider_names.len()
        };
        state.status = format!(
            "0.0%/{} (auto)",
            pi_tui::format_tokens(context_window_of(parts_ref))
        );
    }
    if let Some(parts_ref) = parts.as_ref() {
        // ref `interactive-mode.ts:905-916`：启动屏的 `Model scope:` 行来自 scopedModels。
        state.model_scope = if parts_ref.scoped_models_configured {
            parts_ref
                .available_models
                .iter()
                .map(|choice| choice.id.clone())
                .collect()
        } else {
            Vec::new()
        };
        if parts_ref.supports_thinking {
            // ref `ai/src/models.ts:924-931`：`xhigh`/`max` **只有在模型声明了
            // `thinkingLevelMap` 对应项时才出现**；没有映射表的推理模型只有这五档。
            state.available_thinking_levels = ["off", "minimal", "low", "medium", "high"]
                .iter()
                .map(|level| (*level).to_string())
                .collect();
            // ref `sdk.ts:251-254` `clampThinkingLevel(model, level)`：模型能力之外的
            // 档位夹到**支持的最高档**（如模型只到 `medium` 时 `high` → `medium`）。
            let requested = parts_ref
                .initial_thinking_level
                .clone()
                .unwrap_or_else(|| "medium".to_string());
            state.thinking_level =
                clamp_thinking_level(&state.available_thinking_levels, &requested);
        }
    }
    let initial = parts
        .as_ref()
        .map(|parts| parts.initial_messages.clone())
        .unwrap_or_default();
    if !initial.is_empty() {
        state.transcript.extend(transcript_from_messages(&initial));
    }
    // `-c` 续写：footer 的用量/费用要**接着会话累计**（T61 golden：`↑200 ↓40 $0.000`）。
    let resumed_usage = parts.as_ref().and_then(|parts| {
        let path = parts.resumed_path.as_ref()?;
        crate::settings_selector::session_usage(path).ok()
    });
    // 最近一条 assistant 的 `calculateContextTokens(usage)`（含 cache；**不是**历史累加）。
    let last_assistant_input = initial
        .iter()
        .rev()
        .find_map(|message| match message {
            pi_ai::message::Message::Assistant(assistant) => {
                Some(pi_ai::calculate_context_tokens(&assistant.usage))
            }
            other => {
                let _ = other;
                None
            }
        })
        .unwrap_or(0);
    let history: std::sync::Arc<std::sync::Mutex<Vec<pi_ai::message::Message>>> =
        std::sync::Arc::new(std::sync::Mutex::new(initial));
    // 输入线程拿走一份发送端（agent run 需要主线程这份，故 clone 后再 move）。
    let input_tx = tx.clone();
    // P7-6ae/S3：**r3 走原始字节路径**（input.rs → 键名映射），R1/R2 仍走 crossterm。
    // Windows 例外：控制台在 VT 输入模式下不把 `Ctrl+D`/`Ctrl+P` 变成可用字节
    // （真机反馈"ctrl+d 无法退出、ctrl+p 切换不了模型"），改用 crossterm 事件。
    #[cfg(windows)]
    spawn_crossterm_input_thread(input_tx);
    #[cfg(not(windows))]
    spawn_r3_input_thread(input_tx);

    let mut cols = terminal::size().map(|(c, _)| c as usize).unwrap_or(80);
    let mut rows = terminal::size().map(|(_, r)| r as usize).unwrap_or(24);
    // R2 需要上一帧的屏幕行；R1 忽略它。resize 时必须清空（下一帧整屏重画）。
    let mut previous: Vec<String> = Vec::new();
    // P7-6n/W2：`r3` 的差分状态（R1/R2 不使用）。
    let mut diff_state = pi_tui::diff::DiffState::new();
    // P7-6an/U：`showHardwareCursor`（ref `tui.ts:478` 默认 false，由 settings.json 传入，
    // `cli/startup-ui.ts:89`）。开启后 r3 每帧写 `\x1b[?25h` 显示硬件光标。
    // ref `core/settings-manager.ts:1369-1370`：
    //   `this.settings.showHardwareCursor ?? process.env.PI_HARDWARE_CURSOR === "1"`
    // ⇒ settings 优先、环境变量兜底、缺省 false（`tui.ts:478`）。此前这里只在
    // `PI_CODING_AGENT_DIR` 显式设置时才读 settings，默认配置目录下永远拿不到开关。
    let settings_show_hardware_cursor = parts
        .as_ref()
        .and_then(|parts| std::fs::read_to_string(parts.agent_dir.join("settings.json")).ok())
        .and_then(|text| serde_json::from_str::<crate::settings::Settings>(&text).ok())
        .and_then(|settings| settings.show_hardware_cursor);
    diff_state.show_hardware_cursor = resolve_show_hardware_cursor(
        settings_show_hardware_cursor,
        std::env::var("PI_HARDWARE_CURSOR").ok().as_deref(),
    );
    // P7-6am/P'：跨帧复用条目渲染缓存（宽度变化由缓存内部自动失效）。
    let mut render_cache = pi_tui::RenderCache::new();
    let mut dirty = true;
    // P7-6ao/E：**渲染节流**——ref `tui.ts:477` `MIN_RENDER_INTERVAL_MS = 16` +
    // `scheduleRender()`（`:986-998`）把重绘合并到 ≤60fps。我们原来**每个流式增量都整帧重绘**
    // （97KB 回复按 5–20 字节分片 ⇒ 每轮数千帧），多轮后耗时线性叠加 ⇒ 真机"卡死"
    // （实测 turn20 击键延迟 10.0 s，TS 同场景 114 ms）。
    let min_render_interval = Duration::from_millis(16);
    let mut last_draw: Option<std::time::Instant> = None;
    // P7-7a：`/model` 选择器的状态（`None` = 未打开）。
    let mut selector: Option<SelectorUi> = None;
    // `/settings` → 子菜单时保存父列表状态，Esc 时原样回去（ref `closeSubmenu` 恢复到原选中项）。
    let mut settings_parent: Option<(crate::settings_selector::SettingsSelector, usize)> = None;
    // `/tree` 选中条目要放回编辑器的文本（`Summarize branch?` 选完之后用）。
    let mut pending_tree_restore: Option<String> = None;
    // `/tree` 选中条目的 id（`branch_summary` 条目的 `fromId`）。
    let mut pending_tree_restore_id: Option<String> = None;
    // `Summarize with custom prompt` 提交的指令文本（拼进摘要请求）。
    let mut pending_custom_instructions: Option<String> = None;
    // `/tree` 打标签时的目标条目 id（`label` 条目的 `targetId`）。
    let mut pending_label_target: Option<String> = None;
    // 保存 label 后要落回光标的目标条目 id。
    let mut pending_tree_label_target: Option<String> = None;
    // `/settings` 落盘需要配置目录；`parts` 为 `None`（无 agent 装配）时不写。
    let agent_dir = parts.as_ref().map(|parts| parts.agent_dir.clone());
    let agent_dir = agent_dir.as_deref();
    if let Some(totals) = resumed_usage {
        if let Some(parts_ref) = parts.as_ref() {
            let window = pi_tui::format_tokens(context_window_of(parts_ref));
            // ref `agent-session.ts:3858-3902` 的 `estimateContextTokens`：**有 assistant 的 usage 时**
            // 直接用 `calculateContextTokens(usage)`（≈ input，不含 output）当基数；
            // 只有在没有可用 usage 时才退化为逐条估算。
            let window_tokens = context_window_of(parts_ref);
            let percent = if window_tokens == 0 {
                None
            } else if last_assistant_input > 0 {
                Some(last_assistant_input as f64 / window_tokens as f64 * 100.0)
            } else {
                let system_tokens =
                    pi_core::agent::compaction::estimate_tokens(&parts_ref.system_message);
                Some(system_tokens as f64 / window_tokens as f64 * 100.0)
            };
            let _ = apply(
                &mut state,
                TuiEvent::Usage {
                    input: totals.input,
                    output: totals.output,
                    percent,
                    window,
                    auto: true,
                    // ref `footer.ts:143-146`：`if (usageTotals.cost || usingSubscription)`
                    // ⇒ 累计费用为 0 时**不显示** `$0.000`（重放只有 usage、cost 为 0 的会话即此）。
                    cost: if totals.cost == 0.0 {
                        None
                    } else {
                        Some(totals.cost)
                    },
                },
            );
        }
    }
    // **当前回合的取消令牌**：Escape 用它中止正在跑的回合（ref 的 `session.abort()`）。
    // 槽位在提交线程启动回合时填入，回合结束后清空。
    let turn_cancel: std::sync::Arc<std::sync::Mutex<Option<tokio_util::sync::CancellationToken>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    // V6 调试开关：`PI_TUI_CURSOR_DUMP=<路径>`（值为 `1` 时用系统临时目录下的
    // `pi-rs-cursor.log`）→ 每画一帧追加一行光标行列，供与 TS 侧同口径逐帧对比。
    // 未设置时**零开销**（不建文件、不格式化）。
    let mut cursor_dump = cursor_dump_file();
    let mut frame_index: u64 = 0;
    let frame_started = std::time::Instant::now();
    'main: loop {
        if sig_flag.load(Ordering::SeqCst) {
            break;
        }
        // 合并重绘：距上一帧不足 `min_render_interval` 时**不画、也不睡眠**——
        // 直接让下面的 `recv_timeout` 等剩余时间，到点后由本循环再画一帧。
        // （真机反馈"快速输入会卡"的成因之一就是这里原来用 `sleep` 补齐间隔：
        //  每次击键都被强行拖满 16 ms，连打时延迟线性叠加。）
        let render_ready = last_draw.is_none_or(|last| last.elapsed() >= min_render_interval);
        if dirty && render_ready {
            last_draw = Some(std::time::Instant::now());
            draw(
                &state,
                cols,
                rows,
                &mut previous,
                &mut diff_state,
                &mut render_cache,
            )?;
            if let Some(file) = cursor_dump.as_mut() {
                frame_index += 1;
                let elapsed_ms = frame_started.elapsed().as_millis() as u64;
                write_cursor_dump(
                    file,
                    frame_index,
                    elapsed_ms,
                    cols,
                    rows,
                    &diff_state,
                    &state,
                );
            }
            dirty = false;
        }
        // P7-6ao/E：**合并同一批已排队的事件**（流式增量常一次到几十上百个）。
        // 若每个事件都画一帧，200 delta 的一轮回复就要画 200 帧，每帧 O(历史行数) ⇒ 多轮后数秒级延迟；
        // ref 用 `MIN_RENDER_INTERVAL_MS = 16` 的**定时器合并**（`tui.ts:477/986-998`）达到同样效果。
        let mut wait = if dirty {
            last_draw
                .map(|last| min_render_interval.saturating_sub(last.elapsed()))
                .unwrap_or(Duration::ZERO)
                .max(Duration::from_millis(1))
        } else {
            Duration::from_millis(200)
        };
        // 运行中指示器要**动**：ref 是 `setInterval(80ms)` 轮换 `⠋⠙⠹…`（`loader.ts:13-14/79-86`）。
        // 这里把等待时间夹到 `SPINNER_INTERVAL`，超时即推进一帧并重绘。
        if state.working {
            wait = wait.min(pi_tui::SPINNER_INTERVAL);
        }
        let first_event = match rx.recv_timeout(wait) {
            Ok(event) => Some(event),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if state.working {
                    state.tick_spinner();
                    dirty = true;
                }
                None
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut batch: Vec<TuiEvent> = first_event.into_iter().collect();
        while let Ok(extra) = rx.try_recv() {
            batch.push(extra);
        }
        for event in batch {
            // `Working(true)` 必须**立刻可见**：TS 在回合开始时就点亮指示器，
            // 而我们的批量合并只在 16ms 间隔到点才画，快回合会整段跳过该帧
            // （T05 录制里连 `Working` 行都没有）。这里把上一帧时间清掉，
            // 让循环顶部立刻满足 `render_ready`、先画一帧再继续处理本批事件。
            if matches!(event, TuiEvent::Working(true)) {
                last_draw = None;
            }
            // `/thinking` 选择器里的 `ctrl+s` = "设为默认"（ref 选择器提示行
            // `Enter to select · Ctrl+S to set as default`；`interactive-mode.ts:5012-5030`
            // 的 `onSelectAsDefault` → `selectThinkingLevel(level, true)` → 状态行
            // `Default thinking level: <level>`）。
            if matches!(event, TuiEvent::CtrlS) {
                if let Some(SelectorUi::Thinking(sel)) = selector.as_ref() {
                    if let Some(level) = sel.selected_level() {
                        if let Some(parts_ref) = parts.as_ref() {
                            let _ = crate::settings::persist_setting(
                                &parts_ref.agent_dir,
                                "defaultThinkingLevel",
                                serde_json::Value::String(level.clone()),
                            );
                            if let Some(writer) = parts_ref.writer.as_ref() {
                                if let Ok(mut guard) = writer.lock() {
                                    let _ = guard.append_thinking_level_change(&level);
                                }
                            }
                        }
                        state.thinking_level.clone_from(&level);
                        let _ = apply(
                            &mut state,
                            TuiEvent::Status(format!("Default thinking level: {level}")),
                        );
                        selector = None;
                        state.close_panel();
                        dirty = true;
                        continue;
                    }
                }
            }
            // P7-7a：`/model` 选择器打开时，按键先交给选择器（ref `model-selector.ts:370-405`
            // 的 `handleInput`：先 Tab/上下/Enter/Esc/Ctrl+S，其余**全部**进搜索框）。
            if let Some(open_selector) = selector.as_mut() {
                match &event {
                    // 选择器打开时也可能收到 provider 错误（后台回合还在跑）：
                    // 这类非按键事件**照常进会话区**，不参与选择器的按键分派。
                    TuiEvent::BackTab => {
                        // `/thinking` 选择器里 shift+tab 与 tab 同义（上下移动）；
                        // 其它选择器忽略这个键。
                        // 选择器内部的上下移动由  负责；这里只吞掉该键。
                        dirty = true;
                        continue;
                    }
                    TuiEvent::AssistantNotice(_)
                    | TuiEvent::ThemeName(_)
                    | TuiEvent::ClearStatus
                    | TuiEvent::CtrlZ
                    | TuiEvent::ToolResult { .. }
                    | TuiEvent::RetryState(_)
                    | TuiEvent::AssistantDelta(_)
                    | TuiEvent::ThinkingDelta(_)
                    | TuiEvent::ToolCall { .. }
                    | TuiEvent::Usage { .. }
                    | TuiEvent::UsageTotals { .. }
                    | TuiEvent::Status(_)
                    | TuiEvent::BranchSummary(_)
                    | TuiEvent::Working(_) => {
                        let _ = apply(&mut state, event.clone());
                        dirty = true;
                        continue;
                    }
                    TuiEvent::PageUp | TuiEvent::PageDown => {
                        // `/resume`：整页跳（ref `session-selector.ts:621-626`，
                        // `tui.select.pageUp/pageDown` 默认键 = pageUp/pageDown）。
                        if let SelectorUi::Resume(sel, cols) = open_selector {
                            if matches!(event, TuiEvent::PageUp) {
                                sel.page_up(crate::settings_selector::ResumeSelector::MAX_VISIBLE);
                            } else {
                                sel.page_down(
                                    crate::settings_selector::ResumeSelector::MAX_VISIBLE,
                                );
                            }
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                        }
                        dirty = true;
                        continue;
                    }
                    TuiEvent::CursorLeft | TuiEvent::CursorRight
                        if matches!(open_selector, SelectorUi::Tree(_, _)) =>
                    {
                        // ref `:1026-1031`：`←/→` 是**翻页**（按终端行数步进，夹在两端）。
                        if let SelectorUi::Tree(sel, cols) = open_selector {
                            // T61 golden 实测：40 条条目时 40→25→10，步进 **15**（= rows/2），
                            // 即这里的翻页是**半页**（ref `:1026-1031` 用 maxVisibleLines，
                            // 而组件实际拿到的是可用高度的一半）。
                            let step = rows / 2;
                            if matches!(event, TuiEvent::CursorLeft) {
                                sel.page_up(step);
                            } else {
                                sel.page_down(step);
                            }
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                        }
                        continue;
                    }
                    TuiEvent::CtrlLeft | TuiEvent::CtrlRight
                        if matches!(open_selector, SelectorUi::Tree(_, _)) =>
                    {
                        // ref `:1010-1025`：折叠/展开或分支跳转。
                        if let SelectorUi::Tree(sel, cols) = open_selector {
                            if matches!(event, TuiEvent::CtrlLeft) {
                                sel.fold_or_up();
                            } else {
                                sel.unfold_or_down();
                            }
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                        }
                        continue;
                    }
                    TuiEvent::CtrlO
                    | TuiEvent::CtrlShiftO
                    | TuiEvent::CtrlT
                    | TuiEvent::CtrlL
                    | TuiEvent::CtrlD
                    | TuiEvent::LineStart
                    | TuiEvent::DeleteToLineStart
                        if matches!(open_selector, SelectorUi::Tree(_, _)) =>
                    {
                        // `/tree` 的过滤键（ref `core/keybindings.ts:210-231`）：
                        // `ctrl+o` 循环、`ctrl+t` no-tools、`ctrl+u` user-only、
                        // `ctrl+l` labeled-only、`ctrl+a` all、`ctrl+d` 回默认。
                        if let SelectorUi::Tree(sel, cols) = open_selector {
                            match &event {
                                TuiEvent::CtrlO => sel.cycle_filter(),
                                TuiEvent::CtrlShiftO => sel.cycle_filter_back(),
                                TuiEvent::CtrlT => sel
                                    .set_filter(crate::settings_selector::TreeFilterMode::NoTools),
                                TuiEvent::DeleteToLineStart => sel
                                    .set_filter(crate::settings_selector::TreeFilterMode::UserOnly),
                                TuiEvent::CtrlL => sel.set_filter(
                                    crate::settings_selector::TreeFilterMode::LabeledOnly,
                                ),
                                TuiEvent::LineStart => {
                                    sel.set_filter(crate::settings_selector::TreeFilterMode::All)
                                }
                                TuiEvent::CtrlD => sel.reset_filter(),
                                TuiEvent::Char(_)
                                | TuiEvent::ThemeName(_)
                                | TuiEvent::CtrlP
                                | TuiEvent::CtrlS
                                | TuiEvent::CtrlN
                                | TuiEvent::CtrlR
                                | TuiEvent::CtrlLeft
                                | TuiEvent::CtrlRight
                                | TuiEvent::CtrlX
                                | TuiEvent::Backspace
                                | TuiEvent::Enter
                                | TuiEvent::Escape
                                | TuiEvent::CursorLeft
                                | TuiEvent::CursorRight
                                | TuiEvent::LineEnd
                                | TuiEvent::Delete
                                | TuiEvent::DeleteToLineEnd
                                | TuiEvent::DeleteWordBackward
                                | TuiEvent::Tab
                                | TuiEvent::BackTab
                                | TuiEvent::Newline
                                | TuiEvent::HistoryPrev
                                | TuiEvent::HistoryNext
                                | TuiEvent::PageUp
                                | TuiEvent::PageDown
                                | TuiEvent::Clear
                                | TuiEvent::Paste(_)
                                | TuiEvent::Quit
                                | TuiEvent::AssistantNotice(_)
                                | TuiEvent::ClearStatus
                                | TuiEvent::CtrlZ
                                | TuiEvent::ToolResult { .. }
                                | TuiEvent::RetryState(_)
                                | TuiEvent::AssistantDelta(_)
                                | TuiEvent::ThinkingDelta(_)
                                | TuiEvent::ToolCall { .. }
                                | TuiEvent::Usage { .. }
                                | TuiEvent::UsageTotals { .. }
                                | TuiEvent::Status(_)
                                | TuiEvent::BranchSummary(_)
                                | TuiEvent::Working(_) => {}
                            }
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                        }
                        continue;
                    }
                    TuiEvent::Char('T') if matches!(open_selector, SelectorUi::Tree(_, _)) => {
                        // ref `:1098`：`shift+t` 切换"显示标签时间戳"。
                        if let SelectorUi::Tree(sel, cols) = open_selector {
                            sel.toggle_label_timestamps();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                        }
                        continue;
                    }
                    TuiEvent::Char('L') if matches!(open_selector, SelectorUi::Tree(_, _)) => {
                        // ref `tree-selector.ts:1279-1311`：`shift+l` 打开标签输入。
                        pending_label_target = if let SelectorUi::Tree(sel, _) = open_selector {
                            let visible = sel.visible_indices();
                            visible
                                .get(sel.selected)
                                .map(|index| sel.rows[*index].id.clone())
                        } else {
                            None
                        };
                        let input = crate::settings_selector::LabelInput::new();
                        state.open_panel(pi_tui::PanelPlacement::BelowEditor, input.lines(cols));
                        selector = Some(SelectorUi::Label(input, cols));
                        dirty = true;
                        continue;
                    }
                    TuiEvent::CtrlD
                        if matches!(
                            open_selector,
                            SelectorUi::Resume(sel, _) if !sel.confirming_delete
                        ) =>
                    {
                        // ref `:543-557`：`ctrl+d` 先进入**确认态**（确认态里其它键一律忽略）。
                        if let SelectorUi::Resume(sel, cols) = open_selector {
                            sel.enter_delete_confirm();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                        }
                        continue;
                    }
                    TuiEvent::CtrlR if matches!(open_selector, SelectorUi::Resume(_, _)) => {
                        // ref `session-selector.ts:885-896`：`ctrl+r` 进入重命名模式。
                        if let SelectorUi::Resume(sel, cols) = open_selector {
                            sel.enter_rename();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                        }
                        continue;
                    }
                    TuiEvent::CtrlP if matches!(open_selector, SelectorUi::Resume(_, _)) => {
                        // ref `app.session.togglePath`：`ctrl+p` 切换 `path (off|on)`。
                        if let SelectorUi::Resume(sel, cols) = open_selector {
                            sel.toggle_path();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                        }
                        continue;
                    }
                    TuiEvent::CtrlN if matches!(open_selector, SelectorUi::Resume(_, _)) => {
                        // ref `app.session.toggleNamedFilter`：`ctrl+n` 切换 `Name: All|Named`。
                        if let SelectorUi::Resume(sel, cols) = open_selector {
                            sel.toggle_named();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                        }
                        continue;
                    }
                    TuiEvent::CtrlS if matches!(open_selector, SelectorUi::Resume(_, _)) => {
                        // ref `app.session.toggleSort`：`Sort: Threaded ↔ Recent`。
                        if let SelectorUi::Resume(sel, cols) = open_selector {
                            sel.toggle_sort();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                        }
                        continue;
                    }
                    TuiEvent::Char(ch)
                        if matches!(
                            open_selector,
                            SelectorUi::Summarize(prompt) if prompt.custom.is_some()
                        ) =>
                    {
                        if let SelectorUi::Summarize(prompt) = open_selector {
                            prompt.custom_push(*ch);
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, prompt.lines());
                        }
                        dirty = true;
                        continue;
                    }
                    TuiEvent::Backspace
                        if matches!(
                            open_selector,
                            SelectorUi::Summarize(prompt) if prompt.custom.is_some()
                        ) =>
                    {
                        if let SelectorUi::Summarize(prompt) = open_selector {
                            prompt.custom_pop();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, prompt.lines());
                        }
                        dirty = true;
                        continue;
                    }
                    TuiEvent::Newline
                        if matches!(
                            open_selector,
                            SelectorUi::Summarize(prompt) if prompt.custom.is_some()
                        ) =>
                    {
                        if let SelectorUi::Summarize(prompt) = open_selector {
                            prompt.custom_newline();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, prompt.lines());
                        }
                        dirty = true;
                        continue;
                    }
                    TuiEvent::Escape
                        if matches!(
                            open_selector,
                            SelectorUi::Summarize(prompt) if prompt.custom.is_some()
                        ) =>
                    {
                        // Esc 退出自定义输入态、回到三项菜单（ref `:5439-5442` 重新显示选择器）。
                        if let SelectorUi::Summarize(prompt) = open_selector {
                            prompt.exit_custom();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, prompt.lines());
                        }
                        dirty = true;
                        continue;
                    }
                    TuiEvent::Char(ch) => {
                        // ref `settings-list.ts:239-243`：搜索框为空时**空格**等于"激活当前条目"，
                        // 不当普通字符塞进搜索框。
                        if *ch == ' ' && matches!(open_selector, SelectorUi::Warnings(_)) {
                            if let SelectorUi::Warnings(submenu) = open_selector {
                                let value = submenu.toggle();
                                if let Some(agent_dir) = agent_dir {
                                    let _ = crate::settings::persist_nested_setting(
                                        agent_dir,
                                        "warnings",
                                        "anthropicExtraUsage",
                                        serde_json::Value::Bool(value),
                                    );
                                }
                                state.open_panel(
                                    pi_tui::PanelPlacement::BelowEditor,
                                    submenu.lines(),
                                );
                            }
                            dirty = true;
                            continue;
                        }
                        if *ch == ' '
                            && matches!(open_selector, SelectorUi::Settings(sel, _) if sel.query.is_empty())
                        {
                            let notice = apply_settings_change(
                                &mut state,
                                open_selector.activate(),
                                agent_dir,
                            );
                            if let Some(message) = notice {
                                let _ = apply(&mut state, TuiEvent::Status(message));
                            }
                            // Theme 是子菜单条目（ref `settings-selector.ts:718-724` 的 `submenu`）：
                            // 进子菜单前先存父列表状态。
                            if let SelectorUi::Settings(parent, cols) = open_selector {
                                match parent.pending_submenu.take() {
                                    Some("theme") => {
                                        let submenu = crate::settings_selector::ThemeSubmenu::new(
                                            vec!["dark".to_string(), "light".to_string()],
                                            parent.current_of_theme.clone(),
                                        );
                                        settings_parent = Some((parent.clone(), *cols));
                                        state.open_panel(
                                            pi_tui::PanelPlacement::BelowEditor,
                                            submenu.lines(),
                                        );
                                        selector = Some(SelectorUi::Theme(submenu));
                                        dirty = true;
                                        continue;
                                    }
                                    Some("warnings") => {
                                        let submenu =
                                            crate::settings_selector::WarningsSubmenu::new(true);
                                        settings_parent = Some((parent.clone(), *cols));
                                        state.open_panel(
                                            pi_tui::PanelPlacement::BelowEditor,
                                            submenu.lines(),
                                        );
                                        selector = Some(SelectorUi::Warnings(submenu));
                                        dirty = true;
                                        continue;
                                    }
                                    Some("model-thinking") => {
                                        let submenu =
                                            crate::settings_selector::ModelThinkingSubmenu::new(
                                                vec![(
                                                    parent.last_model_id.clone().unwrap_or_else(
                                                        || "mock-model".to_string(),
                                                    ),
                                                    parent.last_provider_id.clone().unwrap_or_else(
                                                        || "openai-completions".to_string(),
                                                    ),
                                                )],
                                            );
                                        settings_parent = Some((parent.clone(), *cols));
                                        state.open_panel(
                                            pi_tui::PanelPlacement::BelowEditor,
                                            submenu.lines(),
                                        );
                                        selector = Some(SelectorUi::ModelThinking(submenu));
                                        dirty = true;
                                        continue;
                                    }
                                    Some(_) | None => {}
                                }
                            }
                            state.open_panel(
                                pi_tui::PanelPlacement::BelowEditor,
                                open_selector.lines(),
                            );
                            dirty = true;
                            continue;
                        }
                        open_selector.push(*ch);
                        state
                            .open_panel(pi_tui::PanelPlacement::BelowEditor, open_selector.lines());
                        dirty = true;
                        continue;
                    }
                    TuiEvent::Backspace => {
                        open_selector.pop();
                        state
                            .open_panel(pi_tui::PanelPlacement::BelowEditor, open_selector.lines());
                        dirty = true;
                        continue;
                    }
                    TuiEvent::HistoryPrev => {
                        // `/settings` 的上移（ref `SettingsList` 的 `tui.select.up`）；
                        // `/model`/`/thinking` 单条列表移动后内容不变。
                        open_selector.move_up();
                        state
                            .open_panel(pi_tui::PanelPlacement::BelowEditor, open_selector.lines());
                        dirty = true;
                        continue;
                    }
                    TuiEvent::HistoryNext => {
                        open_selector.move_down();
                        state
                            .open_panel(pi_tui::PanelPlacement::BelowEditor, open_selector.lines());
                        dirty = true;
                        continue;
                    }
                    TuiEvent::Tab => {
                        // `/resume`：Tab 切换 Current Folder / All（ref `session-selector.ts:700-712`）。
                        if let SelectorUi::Resume(sel, cols) = open_selector {
                            sel.toggle_scope();
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(*cols));
                            dirty = true;
                            continue;
                        }
                        // 其它选择器：Tab 切 scope（ref `model-selector.ts:372-376`），当前无 scoped 模型。
                        continue;
                    }
                    TuiEvent::Enter => {
                        // `/resume` 的删除确认态：Enter = 确认删除（ref `:545-551`）。
                        if let SelectorUi::Resume(sel, cols) = open_selector {
                            if sel.confirming_delete {
                                if let Some(path) = sel.selected_path() {
                                    let _ = std::fs::remove_file(&path);
                                }
                                sel.exit_delete_confirm();
                                let sessions_dir =
                                    parts.as_ref().map(|parts| parts.session_dir.clone());
                                if let Some(dir) = sessions_dir {
                                    sel.sessions = crate::settings_selector::scan_sessions(&dir);
                                    sel.selected = 0;
                                }
                                state.open_panel(
                                    pi_tui::PanelPlacement::BelowEditor,
                                    sel.lines(*cols),
                                );
                                dirty = true;
                                continue;
                            }
                        }
                        // `/tree`：选中非当前叶子 ⇒ 先问 "Summarize branch?"（ref `:5426-5445`）。
                        if let SelectorUi::Tree(sel, _) = open_selector {
                            let visible = sel.visible_indices();
                            if sel.selected + 1 == visible.len() {
                                // ref `interactive-mode.ts:5416-5420`：选中**当前叶子**时不弹
                                // "Summarize branch?"，而是关掉选择器并给状态行 ` Already at this point`。
                                selector = None;
                                state.close_panel();
                                let _ = apply(
                                    &mut state,
                                    TuiEvent::Status("Already at this point".to_string()),
                                );
                                dirty = true;
                                continue;
                            }
                            if sel.selected + 1 != visible.len() {
                                pending_tree_restore = visible
                                    .get(sel.selected)
                                    .map(|index| sel.rows[*index].text.clone());
                                pending_tree_restore_id = visible
                                    .get(sel.selected)
                                    .map(|index| sel.rows[*index].id.clone());
                                let prompt = crate::settings_selector::SummarizeSelector::new();
                                state.open_panel(
                                    pi_tui::PanelPlacement::BelowEditor,
                                    prompt.lines(),
                                );
                                selector = Some(SelectorUi::Summarize(prompt));
                                dirty = true;
                            }
                            continue;
                        }
                        // `shift+l` 标签输入：Enter 保存并回到树（本项目尚未实现 label 条目落盘）。
                        if let SelectorUi::Label(input, _) = open_selector {
                            if let Some(parts_ref) = parts.as_ref() {
                                if let Some(writer) = parts_ref.writer.as_ref() {
                                    if let Ok(mut guard) = writer.lock() {
                                        let target =
                                            pending_label_target.take().unwrap_or_default();
                                        pending_tree_label_target = Some(target.clone());
                                        let _ = guard.append_label(&target, &input.text);
                                    }
                                }
                            }
                            let rows = parts.as_ref().map_or_else(Vec::new, |parts_ref| {
                                parts_ref
                                    .writer
                                    .as_ref()
                                    .and_then(|writer| {
                                        let path = writer.lock().ok()?.path().to_path_buf();
                                        crate::settings_selector::tree_rows_from_session(&path).ok()
                                    })
                                    .unwrap_or_default()
                            });
                            let mut tree = crate::settings_selector::TreeSelector::new(rows);
                            // 保存后光标仍停在被打标签的那一条（T49 golden）。
                            if let Some(target) = pending_tree_label_target.as_deref() {
                                if let Some(index) =
                                    tree.rows.iter().position(|row| row.id == target)
                                {
                                    let visible = tree.visible_indices();
                                    if let Some(pos) = visible.iter().position(|i| *i == index) {
                                        tree.selected = pos;
                                    }
                                }
                            }
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, tree.lines(cols));
                            selector = Some(SelectorUi::Tree(tree, cols));
                            dirty = true;
                            continue;
                        }
                        // `Summarize branch?` 选完（本切片只实现 `No summary` 分支）。
                        if let SelectorUi::Summarize(prompt) = open_selector {
                            // 第三项 ⇒ 先进入"自定义指令"输入态（ref `:5447-5449`）。
                            if prompt.custom.is_none()
                                && prompt.picked() == "Summarize with custom prompt"
                            {
                                prompt.enter_custom();
                                state.open_panel(
                                    pi_tui::PanelPlacement::BelowEditor,
                                    prompt.lines(),
                                );
                                dirty = true;
                                continue;
                            }
                            let picked = if let Some(text) = prompt.custom.take() {
                                // 自定义指令已提交：记下来，稍后拼进摘要请求文本。
                                pending_custom_instructions = Some(text);
                                "Summarize"
                            } else {
                                prompt.picked()
                            };
                            selector = None;
                            state.close_panel();
                            if picked == "Summarize" {
                                // ref `interactive-mode.ts:3774-3776`：会话区插入 `[branch]` 块；
                                // 先发一次摘要请求（ref 的请求数是 3，我们此前只有 2）。
                                // 请求在后台线程执行，结果通过事件送回（与 agent turn 同一套通道）。
                                if let (Some(parts_ref), Some(text)) =
                                    (parts.as_ref(), pending_tree_restore.clone())
                                {
                                    let provider_now = parts_ref
                                        .model_state
                                        .lock()
                                        .map(|state| {
                                            (
                                                std::sync::Arc::clone(&state.provider),
                                                state.model_id.clone(),
                                            )
                                        })
                                        .ok();
                                    if let Some((provider, model_id)) = provider_now {
                                        let tx2 = tx.clone();
                                        let system = parts_ref.config.system.clone();
                                        let max_tokens = parts_ref.config.max_completion_tokens;
                                        let window = context_window_of(parts_ref);
                                        let system_message_for_percent =
                                            parts_ref.system_message.clone();
                                        let writer_for_summary =
                                            parts_ref.writer.as_ref().map(std::sync::Arc::clone);
                                        let from_id_for_summary = pending_tree_restore_id.clone();
                                        let custom_instructions_for_request =
                                            pending_custom_instructions.take();
                                        let session_path =
                                            parts_ref.writer.as_ref().and_then(|writer| {
                                                writer
                                                    .lock()
                                                    .ok()
                                                    .map(|guard| guard.path().to_path_buf())
                                            });
                                        std::thread::spawn(move || {
                                            let request = pi_ai::provider::ProviderRequest {
                                                model: model_id,
                                                context: pi_ai::context::Context {
                                                    system,
                                                    messages: vec![pi_ai::message::Message::User {
                                                        content: pi_ai::UserContent::Text(
                                                            match custom_instructions_for_request.as_deref() {
                                                                Some(instructions) => format!(
                                                                    "{instructions}\n\nBranch to summarize ends with: {text}"
                                                                ),
                                                                None => format!(
                                                                    "Summarize the branch that ends with: {text}"
                                                                ),
                                                            },
                                                        ),
                                                        timestamp: 0,
                                                        extra: serde_json::Map::new(),
                                                    }],
                                                    ..pi_ai::context::Context::default()
                                                },
                                                max_completion_tokens: max_tokens,
                                                tools: Vec::new(),
                                                thinking: None,
                                                headers: Vec::new(),
                                            };
                                            let runtime =
                                                tokio::runtime::Builder::new_current_thread()
                                                    .enable_all()
                                                    .build();
                                            let Ok(runtime) = runtime else {
                                                return;
                                            };
                                            runtime.block_on(async move {
                                                let Ok(stream) = provider.stream(request).await else {
                                                    return;
                                                };
                                                let mut summary = String::new();
                                                let mut usage = None;
                                                let mut stream = stream;
                                                while let Some(event) = futures::StreamExt::next(&mut stream).await {
                                                    if let pi_ai::types::AssistantEvent::TextDelta {
                                                        delta, ..
                                                    } = event
                                                    {
                                                        summary.push_str(&delta);
                                                    }
                                                }
                                                // 用量取流的**最终消息**（`Stream::final_message`）。
                                                if let Ok(Some(message)) = stream.final_message() {
                                                    usage = Some((
                                                        message.usage.input,
                                                        message.usage.output,
                                                    ));
                                                }
                                                // ref 的摘要正文 = **固定前缀** + 模型输出
                                                // （TS 会话实测 114 字符：前缀 84 + 模型输出）。
                                                let summary = format!(
                                                    "The user explored a different conversation branch before returning here.\nSummary of that exploration:\n\n{summary}"
                                                );
                                                // 写 `branch_summary` 会话条目（ref 同名字段集合）。
                                                if let (Some(writer), Some(from_id)) =
                                                    (writer_for_summary.as_ref(), from_id_for_summary.as_deref())
                                                {
                                                    if let Ok(mut guard) = writer.lock() {
                                                        let _ = guard.append_branch_summary(
                                                            from_id,
                                                            &summary,
                                                            usage.map_or(0, |(input, _)| input),
                                                            usage.map_or(0, |(_, output)| output),
                                                        );
                                                    }
                                                }
                                                let summary_for_percent = summary.clone();
                                                let _ = tx2.send(TuiEvent::BranchSummary(summary));
                                                // 顺序与 TS 一致：先插 `[branch]` 块，再出状态行。
                                                let _ = tx2.send(TuiEvent::Status(
                                                    "Navigated to selected point".to_string(),
                                                ));
                                                if let Some((input, output)) = usage {
                                                    // 累计用量：会话里已写的 assistant 用量 + 这次摘要请求
                                                    // （TS 的 footer 是累计值：T52 golden `↑20 ↓4`）。
                                                    let prior = session_path
                                                        .as_ref()
                                                        .and_then(|path| {
                                                            crate::settings_selector::session_usage(path).ok()
                                                        })
                                                        .unwrap_or_default();
                                                    let input = input + prior.input;
                                                    let output = output + prior.output;
                                                    // 百分比走**上下文估算**口径（系统提示 + 分支消息），
                                                    // 与 T05/T45 的 `system_tokens + message_tokens` 一致；
                                                    // 用 token 计数会得到 0.0%，而 TS golden 是 1.2%。
                                                    let percent = if window == 0 {
                                                        None
                                                    } else {
                                                        // ref `agent-session.ts:3858-3902` 的
                                                        // `estimateProjectedContextTokens`：对整个**投影**
                                                        // （系统提示 + 会话里全部消息）估 token，而不是只估一条消息。
                                                        let mut tokens =
                                                            pi_core::agent::compaction::estimate_tokens(
                                                                &system_message_for_percent,
                                                            );
                                                        if let Some(path) = session_path.as_ref() {
                                                            if let Ok(messages) = crate::settings_selector::replay_messages(path) {
                                                                for (assistant, text) in messages {
                                                                    let message = if assistant {
                                                                        pi_ai::message::Message::Assistant(
                                                                            Box::new(pi_ai::types::AssistantMessage {
                                                                                role: "assistant".to_string(),
                                                                                content: vec![pi_ai::types::Content::Text {
                                                                                    text,
                                                                                    text_signature: None,
                                                                                }],
                                                                                api: String::new(),
                                                                                provider: String::new(),
                                                                                model: String::new(),
                                                                                response_model: None,
                                                                                usage: pi_ai::types::Usage::default(),
                                                                                stop_reason: pi_ai::types::StopReason::Stop,
                                                                                error_message: None,
                                                                                raw_stop_reason: None,
                                                                                end_turn: None,
                                                                                response_id: None,
                                                                                timestamp: 0,
                                                                            }),
                                                                        )
                                                                    } else {
                                                                        pi_ai::message::Message::User {
                                                                            content: pi_ai::UserContent::Text(text),
                                                                            timestamp: 0,
                                                                            extra: serde_json::Map::new(),
                                                                        }
                                                                    };
                                                                    tokens +=
                                                                        pi_core::agent::compaction::estimate_tokens(&message);
                                                                }
                                                            }
                                                        }
                                                        // 投影里还要算上刚写的 `branch_summary` 消息
                                                        // （ref `estimateTokens` 对 branchSummary 按
                                                        // `summary.length` 计；T52 golden 1.2% 正来自这 114 字符）。
                                                        tokens += pi_core::agent::compaction::estimate_tokens(
                                                            &pi_ai::message::Message::BranchSummary {
                                                                summary: summary_for_percent.clone(),
                                                                from_id: None,
                                                                timestamp: 0,
                                                                extra: serde_json::Map::new(),
                                                            },
                                                        );
                                                        Some(tokens as f64 / window as f64 * 100.0)
                                                    };
                                                    let _ = tx2.send(TuiEvent::Usage {
                                                        input,
                                                        output,
                                                        percent,
                                                        window: if window >= 1000 {
                                                            format!("{}k", window / 1000)
                                                        } else {
                                                            window.to_string()
                                                        },
                                                        auto: true,
                                                        cost: None,
                                                    });
                                                }
                                            });
                                        });
                                    }
                                }
                                // 只清屏 + 文本回编辑器；`[branch]` 块与状态行由后台摘要线程按序发出。
                                state.transcript.clear();
                                if let Some(text) = pending_tree_restore.take() {
                                    state.input = text;
                                }
                            }
                            if picked == "No summary" {
                                // ref 的结果：会话区只留一行状态行，选中点之后的输入回到编辑器
                                // （T45 golden：会话区是 ` Navigated to selected point`，编辑器里是 `say hi`）。
                                state.transcript.clear();
                                let _ = apply(
                                    &mut state,
                                    TuiEvent::Status("Navigated to selected point".to_string()),
                                );
                                if let Some(text) = pending_tree_restore.take() {
                                    // footer 的百分比在恢复后**按剩余上下文重估**：
                                    // 系统提示 + 该条消息的估算 token（与 T05 同一口径，见下方
                                    // `TuiEvent::Usage` 分支的 `system_tokens + text_tokens`）。
                                    if let Some(parts) = parts.as_ref() {
                                        // ref 的投影用的是**会话里存的 system 条目**，不是启动时
                                        // 重新组装的 `parts.system_message`——两者段落长度可能不同
                                        // （T52 golden：TS 1.2% vs 我们按重组消息算的 1.1%，差 ≈128 token）。
                                        let session_system = parts
                                            .writer
                                            .as_ref()
                                            .and_then(|writer| {
                                                let path = writer.lock().ok()?.path().to_path_buf();
                                                pi_core::session_io::SessionFile::read(&path).ok()
                                            })
                                            .and_then(|file| {
                                                file.entries.into_iter().find_map(|entry| match entry {
                                                    pi_core::session::SessionEntry::Message { message, .. } => {
                                                        match message {
                                                            pi_ai::message::Message::System { .. } => Some(message),
                                                            other => {
                                                                let _ = other;
                                                                None
                                                            }
                                                        }
                                                    }
                                                    other => {
                                                        let _ = other;
                                                        None
                                                    }
                                                })
                                            })
                                            .unwrap_or_else(|| parts.system_message.clone());
                                        let system_tokens =
                                            pi_core::agent::compaction::estimate_tokens(
                                                &session_system,
                                            );
                                        let message_tokens =
                                            pi_core::agent::compaction::estimate_tokens(
                                                &pi_ai::message::Message::User {
                                                    content: pi_ai::UserContent::Text(text.clone()),
                                                    timestamp: 0,
                                                    extra: serde_json::Map::new(),
                                                },
                                            );
                                        let window_tokens = context_window_of(parts);
                                        let window = if window_tokens >= 1000 {
                                            format!("{}k", window_tokens / 1000)
                                        } else {
                                            window_tokens.to_string()
                                        };
                                        let percent = if window_tokens == 0 {
                                            None
                                        } else {
                                            Some(
                                                (system_tokens + message_tokens) as f64
                                                    / window_tokens as f64
                                                    * 100.0,
                                            )
                                        };
                                        let _ = apply(
                                            &mut state,
                                            TuiEvent::Usage {
                                                input: 10,
                                                output: 2,
                                                percent,
                                                window,
                                                auto: true,
                                                cost: None,
                                            },
                                        );
                                    }
                                    state.input = text;
                                }
                            }
                            dirty = true;
                            continue;
                        }
                        // `/resume`：选中会话 ⇒ 重放它的 user/assistant 文本、把写器切到该文件、
                        // 追一条 ` Resumed session` 状态行（ref `interactive-mode.ts:5563-5605`）。
                        if let SelectorUi::Resume(sel, _) = open_selector {
                            if let Some(&index) = sel.visible_indices().get(sel.selected) {
                                let path = sel.sessions[index].path.clone();
                                match crate::settings_selector::replay_messages(&path) {
                                    Ok(messages) => {
                                        state.transcript.clear();
                                        for (assistant, text) in messages {
                                            state.transcript.push(if assistant {
                                                pi_tui::TranscriptItem::Assistant(text)
                                            } else {
                                                pi_tui::TranscriptItem::User(text)
                                            });
                                        }
                                        // 写器切换到该会话文件（追加模式）：后续消息写进被 resume 的文件。
                                        if let Some(parts) = parts.as_ref() {
                                            if let Some(writer) = parts.writer.as_ref() {
                                                if let Ok(opened) =
                                                    crate::session_writer::SessionWriter::open(
                                                        &path,
                                                    )
                                                {
                                                    if let Ok(mut guard) = writer.lock() {
                                                        *guard = opened;
                                                    }
                                                }
                                            }
                                        }
                                        // 从被 resume 的会话累加 usage（ref 的 footer 会显示
                                        // `↑10 ↓2 $0.000 …`：输入/输出/费用都来自会话里的 assistant 消息）。
                                        if let Some(parts) = parts.as_ref() {
                                            let window_tokens = context_window_of(parts);
                                            if let Ok(totals) =
                                                crate::settings_selector::session_usage(&path)
                                            {
                                                let _ = apply(
                                                    &mut state,
                                                    TuiEvent::Usage {
                                                        input: totals.input,
                                                        output: totals.output,
                                                        percent: Some(0.0),
                                                        window: format!(
                                                            "{}k",
                                                            window_tokens / 1000
                                                        ),
                                                        auto: true,
                                                        cost: if totals.cost == 0.0 {
                                                            None
                                                        } else {
                                                            Some(totals.cost)
                                                        },
                                                    },
                                                );
                                            }
                                        }
                                        let _ = apply(
                                            &mut state,
                                            TuiEvent::Status("Resumed session".to_string()),
                                        );
                                        selector = None;
                                        state.close_panel();
                                        dirty = true;
                                        continue;
                                    }
                                    Err(_) => {
                                        // 读不了就保持选择器打开（不静默清屏）。
                                        continue;
                                    }
                                }
                            }
                            continue;
                        }
                        // 两步子菜单：第 1 步进第 2 步；第 2 步落盘并按 `{loop:true}` 回到第 1 步。
                        if let SelectorUi::ModelThinking(submenu) = open_selector {
                            if let Some((index, level)) =
                                submenu.activate().filter(|_| submenu.step.is_some())
                            {
                                let (id, provider) = submenu.models[index].clone();
                                if let Some(agent_dir) = agent_dir {
                                    let _ = crate::settings::persist_nested_setting(
                                        agent_dir,
                                        "modelThinkingLevels",
                                        &format!("{provider}/{id}"),
                                        serde_json::Value::String(level.to_string()),
                                    );
                                }
                            }
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, submenu.lines());
                            dirty = true;
                            continue;
                        }
                        // Warnings 子菜单：Enter/空格都切换布尔并写盘（ref `settings-list.ts:239-243`）。
                        if let SelectorUi::Warnings(submenu) = open_selector {
                            let value = submenu.toggle();
                            if let Some(agent_dir) = agent_dir {
                                let _ = crate::settings::persist_nested_setting(
                                    agent_dir,
                                    "warnings",
                                    "anthropicExtraUsage",
                                    serde_json::Value::Bool(value),
                                );
                            }
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, submenu.lines());
                            dirty = true;
                            continue;
                        }
                        // Theme 子菜单：选中当前主题 → 写盘 → 回到父列表（ref `SelectSubmenu` 的 onSelect）。
                        if let SelectorUi::Theme(submenu) = open_selector {
                            let picked = submenu.picked();
                            if let (Some(theme), Some(agent_dir)) = (picked.clone(), agent_dir) {
                                let _ = crate::settings::persist_setting(
                                    agent_dir,
                                    "theme",
                                    serde_json::Value::String(theme.clone()),
                                );
                                // ref `setTheme`（`settings-manager.ts:793-797`）：
                                // 写配置后**立即重绘**，不用重启。
                                let _ = apply(&mut state, TuiEvent::ThemeName(theme));
                            }
                            if let Some((parent, cols)) = settings_parent.take() {
                                state.open_panel(
                                    pi_tui::PanelPlacement::BelowEditor,
                                    parent.lines(cols),
                                );
                                selector = Some(SelectorUi::Settings(parent, cols));
                            } else {
                                selector = None;
                                state.close_panel();
                            }
                            dirty = true;
                            continue;
                        }
                        // `/settings` 里 Enter = 激活（ref `:239-243`）；`/model`、`/thinking` 里 = 选中并关闭。
                        if open_selector.is_settings() {
                            let notice = apply_settings_change(
                                &mut state,
                                open_selector.activate(),
                                agent_dir,
                            );
                            if let Some(message) = notice {
                                let _ = apply(&mut state, TuiEvent::Status(message));
                            }
                            if let SelectorUi::Settings(parent, cols) = open_selector {
                                match parent.pending_submenu.take() {
                                    Some("theme") => {
                                        let submenu = crate::settings_selector::ThemeSubmenu::new(
                                            vec!["dark".to_string(), "light".to_string()],
                                            parent.current_of_theme.clone(),
                                        );
                                        settings_parent = Some((parent.clone(), *cols));
                                        state.open_panel(
                                            pi_tui::PanelPlacement::BelowEditor,
                                            submenu.lines(),
                                        );
                                        selector = Some(SelectorUi::Theme(submenu));
                                        dirty = true;
                                        continue;
                                    }
                                    Some("warnings") => {
                                        let submenu =
                                            crate::settings_selector::WarningsSubmenu::new(true);
                                        settings_parent = Some((parent.clone(), *cols));
                                        state.open_panel(
                                            pi_tui::PanelPlacement::BelowEditor,
                                            submenu.lines(),
                                        );
                                        selector = Some(SelectorUi::Warnings(submenu));
                                        dirty = true;
                                        continue;
                                    }
                                    Some("model-thinking") => {
                                        let submenu =
                                            crate::settings_selector::ModelThinkingSubmenu::new(
                                                vec![(
                                                    parent.last_model_id.clone().unwrap_or_else(
                                                        || "mock-model".to_string(),
                                                    ),
                                                    parent.last_provider_id.clone().unwrap_or_else(
                                                        || "openai-completions".to_string(),
                                                    ),
                                                )],
                                            );
                                        settings_parent = Some((parent.clone(), *cols));
                                        state.open_panel(
                                            pi_tui::PanelPlacement::BelowEditor,
                                            submenu.lines(),
                                        );
                                        selector = Some(SelectorUi::ModelThinking(submenu));
                                        dirty = true;
                                        continue;
                                    }
                                    Some(_) | None => {}
                                }
                            }
                            state.open_panel(
                                pi_tui::PanelPlacement::BelowEditor,
                                open_selector.lines(),
                            );
                            dirty = true;
                            continue;
                        }
                        // `/thinking` 选中：设置档位（ref `interactive-mode.ts:5001-5010`：
                        // `Thinking level: <level>`，不写 settings）。
                        if let SelectorUi::Thinking(sel) = open_selector {
                            if let Some(level) = sel.selected_level() {
                                state.thinking_level.clone_from(&level);
                                if let Some(parts_ref) = parts.as_ref() {
                                    if let Some(writer) = parts_ref.writer.as_ref() {
                                        if let Ok(mut guard) = writer.lock() {
                                            let _ = guard.append_thinking_level_change(&level);
                                        }
                                    }
                                }
                                let _ = apply(
                                    &mut state,
                                    TuiEvent::Status(format!("Thinking level: {level}")),
                                );
                                selector = None;
                                state.close_panel();
                                dirty = true;
                                continue;
                            }
                        }
                        // `/model` 选中：切换模型 + 会话追加 `model_change` + 状态行
                        // ` Model: <name>`（ref `interactive-mode.ts` 的 model select 路径）。
                        let mut picked: Option<(String, String)> = None;
                        if let SelectorUi::Model(sel) = open_selector {
                            picked = sel
                                .selected_model()
                                .map(|model| (model.provider.clone(), model.id.clone()));
                        }
                        if let (Some((pick_provider, model_id)), Some(parts_ref)) =
                            (picked, parts.as_ref())
                        {
                            match rebuild_provider(parts_ref, &pick_provider, &model_id) {
                                Ok(built) => {
                                    let provider_name = pick_provider.clone();
                                    if let Ok(mut guard) = parts_ref.model_state.lock() {
                                        guard.model_id.clone_from(&model_id);
                                        guard.provider_name.clone_from(&pick_provider);
                                        guard.provider = built.provider;
                                        guard.context_window = built.context_window;
                                    }
                                    refresh_model_window(&mut state, parts_ref, &history);
                                    if let Some(writer) = parts_ref.writer.as_ref() {
                                        if let Ok(mut guard) = writer.lock() {
                                            let _ = guard
                                                .append_model_change(&provider_name, &model_id);
                                        }
                                    }
                                    state.model = model_id.clone();
                                    state.model_provider = provider_name.clone();
                                    let _ = apply(
                                        &mut state,
                                        TuiEvent::Status(format!("Model: {model_id}")),
                                    );
                                }
                                Err(error) => {
                                    let _ = apply(
                                        &mut state,
                                        TuiEvent::Status(format!("无法切换模型：{error}")),
                                    );
                                }
                            }
                        }
                        selector = None;
                        state.close_panel();
                        dirty = true;
                        continue;
                    }
                    TuiEvent::Escape | TuiEvent::CtrlD => {
                        // `/resume` 确认态下 Esc 只取消确认（ref `:552-555`）。
                        if let SelectorUi::Resume(sel, cols) = open_selector {
                            if sel.confirming_delete {
                                sel.exit_delete_confirm();
                                state.open_panel(
                                    pi_tui::PanelPlacement::BelowEditor,
                                    sel.lines(*cols),
                                );
                                dirty = true;
                                continue;
                            }
                        }
                        // 子菜单里 Esc 只回到父列表（ref `SelectSubmenu` 的 onCancel ⇒ `closeSubmenu`）。
                        if let SelectorUi::ModelThinking(submenu) = open_selector {
                            if submenu.back() {
                                state.open_panel(
                                    pi_tui::PanelPlacement::BelowEditor,
                                    submenu.lines(),
                                );
                                dirty = true;
                                continue;
                            }
                        }
                        if open_selector.is_submenu() {
                            if let Some((parent, cols)) = settings_parent.take() {
                                state.open_panel(
                                    pi_tui::PanelPlacement::BelowEditor,
                                    parent.lines(cols),
                                );
                                selector = Some(SelectorUi::Settings(parent, cols));
                                dirty = true;
                                continue;
                            }
                        }
                        selector = None;
                        state.close_panel();
                        dirty = true;
                        continue;
                    }
                    // ref `:404-405`：其余按键**全部**进搜索框。搜索框支持粘贴，
                    // 光标移动在单行查询里无可见效果 ⇒ 两者分别处理，其余显式列为"吞掉"。
                    TuiEvent::Paste(text) => {
                        for ch in text.chars() {
                            if !ch.is_control() {
                                open_selector.push(ch);
                            }
                        }
                        state
                            .open_panel(pi_tui::PanelPlacement::BelowEditor, open_selector.lines());
                        dirty = true;
                        continue;
                    }
                    TuiEvent::CursorLeft
                    | TuiEvent::CursorRight
                    | TuiEvent::LineStart
                    | TuiEvent::LineEnd
                    | TuiEvent::Delete
                    | TuiEvent::DeleteToLineEnd
                    | TuiEvent::DeleteToLineStart
                    | TuiEvent::DeleteWordBackward
                    | TuiEvent::Newline
                    | TuiEvent::Clear
                    | TuiEvent::Quit
                    | TuiEvent::CtrlO
                    | TuiEvent::CtrlT
                    | TuiEvent::CtrlL
                    | TuiEvent::CtrlP
                    | TuiEvent::CtrlShiftO
                    | TuiEvent::CtrlS
                    | TuiEvent::CtrlN
                    | TuiEvent::CtrlR
                    | TuiEvent::CtrlLeft
                    | TuiEvent::CtrlRight
                    | TuiEvent::CtrlX => continue,
                }
            }
            // P7-7f：`ctrl+x` 复制最后一条助手消息（ref `interactive-mode.ts:6377-6404`）：
            // 有网/桌面环境用系统剪贴板，无显示环境退化为 OSC 52；状态行
            // ` Copied last agent message to clipboard`。这里统一走 OSC 52（无需额外依赖）。
            if std::env::var_os("PI_TUI_DUMP_TRANSCRIPT").is_some() {
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/p78c-transcript.log")
                {
                    use std::io::Write as _;
                    let summary: Vec<String> = state
                        .transcript
                        .iter()
                        .map(|item| match item {
                            pi_tui::TranscriptItem::User(text) => {
                                format!("U:{}", text.replace('\n', "~"))
                            }
                            pi_tui::TranscriptItem::Assistant(text) => {
                                format!("A:{}", text.replace('\n', "~"))
                            }
                            other => format!("{other:?}"),
                        })
                        .collect();
                    let _ = writeln!(file, "{}", summary.join(" | "));
                }
            }
            // `app.thinking.save` = `ctrl+s`（ref `keybindings.ts:104-107`）：把当前档位写进
            // settings.json 的 `defaultThinkingLevel`，并补一条 `thinking_level_change` 会话条目。
            if matches!(event, TuiEvent::CtrlS) && selector.is_none() {
                let level = state.thinking_level.clone();
                if let Some(parts_ref) = parts.as_ref() {
                    match crate::settings::persist_setting(
                        &parts_ref.agent_dir,
                        "defaultThinkingLevel",
                        serde_json::Value::String(level.clone()),
                    ) {
                        Ok(()) => {
                            if let Some(writer) = parts_ref.writer.as_ref() {
                                if let Ok(mut guard) = writer.lock() {
                                    let _ = guard.append_thinking_level_change(&level);
                                }
                            }
                            let _ = apply(
                                &mut state,
                                TuiEvent::Status(format!("Thinking level saved: {level}")),
                            );
                        }
                        Err(error) => {
                            let _ = apply(&mut state, TuiEvent::Status(error));
                        }
                    }
                }
                dirty = true;
                continue;
            }
            if matches!(event, TuiEvent::CtrlX) && selector.is_none() {
                let last = state.transcript.iter().rev().find_map(|item| match item {
                    pi_tui::TranscriptItem::Assistant(text) => Some(text.clone()),
                    pi_tui::TranscriptItem::User(_)
                    | pi_tui::TranscriptItem::Thinking(_)
                    | pi_tui::TranscriptItem::Notice(_)
                    | pi_tui::TranscriptItem::Status(_)
                    | pi_tui::TranscriptItem::BranchSummary { .. }
                    | pi_tui::TranscriptItem::ToolCall { .. } => None,
                });
                match last {
                    Some(text) => {
                        // 剪贴板策略见 `crate::clipboard`（平台命令 → OSC 52 → 报错），
                        // 与 ref `utils/clipboard.ts:74-140` 一致。
                        match crate::clipboard::write_to_clipboard(&text) {
                            Ok(()) => {
                                let _ = apply(
                                    &mut state,
                                    TuiEvent::Status(
                                        "Copied last agent message to clipboard".to_string(),
                                    ),
                                );
                            }
                            Err(error) => {
                                let _ = apply(&mut state, TuiEvent::Status(error));
                            }
                        }
                    }
                    None => {
                        let _ = apply(
                            &mut state,
                            TuiEvent::Status("No agent messages to copy yet.".to_string()),
                        );
                    }
                }
                dirty = true;
                continue;
            }
            // P7-7f M：`Ctrl+P` 循环模型（ref `interactive-mode.ts:4378-4394`）；
            // 选择器打开时由选择器自己消费，这里只在没有选择器时生效。
            if matches!(event, TuiEvent::CtrlP) && selector.is_none() {
                if let Some(parts_ref) = parts.as_ref() {
                    let current = parts_ref
                        .model_state
                        .lock()
                        .map(|state| state.model_id.clone())
                        .unwrap_or_default();
                    let ids = parts_ref.available_models.clone();
                    let current_provider = parts_ref
                        .model_state
                        .lock()
                        .map(|state| state.provider_name.clone())
                        .unwrap_or_default();
                    let next = next_switchable_model(&ids, &current, &current_provider, |choice| {
                        rebuild_provider(parts_ref, &choice.provider, &choice.id)
                    });
                    match next {
                        NextModel::OnlyOne => {
                            let _ = apply(
                                &mut state,
                                TuiEvent::Status(
                                    if parts_ref.scoped_models_configured {
                                        "Only one model in scope"
                                    } else {
                                        "Only one model available"
                                    }
                                    .to_string(),
                                ),
                            );
                        }
                        NextModel::NoneSwitchable(error) => {
                            let _ = apply(
                                &mut state,
                                TuiEvent::Status(format!("无法切换模型：{error}")),
                            );
                        }
                        NextModel::Chosen(next_id, built) => {
                            if let Ok(mut guard) = parts_ref.model_state.lock() {
                                guard.model_id.clone_from(&next_id.id);
                                guard.provider_name.clone_from(&next_id.provider);
                                guard.provider = built.provider;
                                guard.context_window = built.context_window;
                            }
                            refresh_model_window(&mut state, parts_ref, &history);
                            if let Some(writer) = parts_ref.writer.as_ref() {
                                if let Ok(mut guard) = writer.lock() {
                                    let _ =
                                        guard.append_model_change(&next_id.provider, &next_id.id);
                                }
                            }
                            state.model = next_id.id.clone();
                            state.model_provider = next_id.provider.clone();
                            // ref `interactive-mode.ts:4387-4389`：切换后按新模型重置思考档位
                            // （推理模型默认 medium、其余 off），并在提示里带上档位。
                            let reasoning = crate::list_models::model_is_reasoning(
                                &state.agent_dir,
                                &next_id.provider,
                                &next_id.id,
                            );
                            state.thinking_level = if reasoning {
                                "medium".to_string()
                            } else {
                                "off".to_string()
                            };
                            let thinking_str = if reasoning && state.thinking_level != "off" {
                                format!(" (thinking: {})", state.thinking_level)
                            } else {
                                String::new()
                            };
                            let _ = apply(
                                &mut state,
                                TuiEvent::Status(format!(
                                    "Switched to {}{thinking_str}",
                                    next_id.id
                                )),
                            );
                        }
                    }
                    dirty = true;
                    continue;
                }
            }
            // P7-7f/perf：一个回合结束（`Working(false)`）⇒ 若还有排队输入，立刻提交下一个。
            if matches!(event, TuiEvent::Working(false)) {
                // 重试退避期间 `AgentSettled` 也会来：整轮没结束，保持"忙"。
                if !state.retrying {
                    busy = false;
                }
                if let Some(next) = queued_inputs.pop_front() {
                    state.input = next;
                    let _ = apply(&mut state, TuiEvent::Enter);
                }
                dirty = true;
            }
            // Escape 中止正在执行的回合（ref `interactive-mode.ts:3607-3613`
            // `escape` → `session.abort()`；重试倒计时期间同样生效）。
            if matches!(event, TuiEvent::Escape) && busy && selector.is_none() {
                let token = turn_cancel.lock().ok().and_then(|slot| {
                    slot.as_ref()
                        .map(tokio_util::sync::CancellationToken::clone)
                });
                if let Some(token) = token {
                    token.cancel();
                    dirty = true;
                    continue;
                }
            }
            // `app.model.select`（ref `keybindings.ts:116`，默认 `ctrl+l`）：
            // 打开模型选择器并把按键**改写成"/model + Enter"**，交给同一套命令分发。
            // `Model catalogs refreshed.` 是**面板内部**的一行（`/model` 构造选择器时
            // 已放进 `refresh_status`），不能再往 transcript push（会整体下移 2 行）。
            let mut event = event;
            if matches!(event, TuiEvent::CtrlL)
                && selector.is_none()
                && !busy
                && state.input.is_empty()
            {
                state.input = "/model".to_string();
                state.cursor = "/model".chars().count();
                event = TuiEvent::Enter;
                dirty = true;
            }
            // `app.suspend`（ref `keybindings.ts:96-99`，非 Windows 默认 `ctrl+z`）：
            // 恢复终端 → 给自己发 SIGTSTP → 回到前台后重新进 raw 并按整屏重画。
            if matches!(event, TuiEvent::CtrlZ) && selector.is_none() && !busy {
                // ref `interactive-mode.ts:4274-4300` `handleCtrlZ()`：
                // 1) 先把光标移到**内容末尾下方**并换行（否则 shell 提示符会接在
                //    最后一行 UI 内容之后，真机表现为 `[1]+ 已停止  ────…` 粘在一起）；
                // 2) 再恢复终端模式（raw/括号粘贴/kitty/光标显示）；
                // 3) 才给自己发 SIGTSTP。
                {
                    use std::io::Write as _;
                    let stop = pi_tui::diff::before_terminal_stop(&diff_state);
                    let mut stdout = std::io::stdout();
                    let _ = stdout.write_all(stop.as_bytes());
                    let _ = stdout.flush();
                }
                guard.restore();
                let pid = std::process::id().to_string();
                // 不引 libc：用系统 `kill` 给自己发 SIGTSTP；`SIGCONT` 后继续执行下一行。
                let _ = std::process::Command::new("kill")
                    .args(["-TSTP", &pid])
                    .status();
                guard = TerminalGuard::new()?;
                // 回前台时终端可能被重绘过：整屏重画。
                // 恢复后屏幕可能被 shell 提示符等内容覆盖：保留记账，但**强制清屏重绘**。
                diff_state.force_clear = true;
                dirty = true;
                continue;
            }
            // `/login` 无参面板：方向键移动、Enter 选定、Esc 取消（ref
            // `interactive-mode.ts:356-402` 的选择器交互；这里复用转录区面板）。
            if let Some((options, index)) = login_panel.as_mut() {
                match &event {
                    pi_tui::TuiEvent::HistoryPrev => {
                        if *index > 0 {
                            *index -= 1;
                        }
                        login_render(&mut state, options, *index);
                        dirty = true;
                        continue;
                    }
                    pi_tui::TuiEvent::HistoryNext => {
                        if *index + 1 < options.len() {
                            *index += 1;
                        }
                        login_render(&mut state, options, *index);
                        dirty = true;
                        continue;
                    }
                    pi_tui::TuiEvent::Escape => {
                        login_panel = None;
                        state.close_panel();
                        dirty = true;
                        continue;
                    }
                    pi_tui::TuiEvent::Enter => {
                        let chosen = options.get(*index).cloned();
                        let providers = state.login_providers.clone();
                        match chosen {
                            // 认证方式层：进入对应类型的 provider 列表
                            Some(item) if item.id.is_none() => {
                                let items = login_provider_items(&providers, item.auth_type);
                                if items.is_empty() {
                                    login_panel = None;
                                    state.close_panel();
                                    // 只在没有内置目录时才会走到这里；同一条提示不重复刷屏
                                    let notice =
                                        "No providers available for that authentication method.";
                                    let already = matches!(
                                        state.transcript.last(),
                                        Some(pi_tui::TranscriptItem::Status(last)) if last == notice
                                    );
                                    if !already {
                                        state.transcript.push(pi_tui::TranscriptItem::Status(
                                            notice.to_string(),
                                        ));
                                    }
                                } else {
                                    *options = items;
                                    *index = 0;
                                    login_render(&mut state, options, 0);
                                }
                            }
                            Some(item) => {
                                login_panel = None;
                                state.close_panel();
                                let name = item
                                    .label
                                    .split(" (")
                                    .next()
                                    .unwrap_or(&item.label)
                                    .to_string();
                                if matches!(item.auth_type, crate::login_options::AuthType::ApiKey)
                                {
                                    let notice = format!("Logging in to {name} (api_key).");
                                    state.pending_login_provider = item.id;
                                    state.input.clear();
                                    state.cursor = 0;
                                    state
                                        .transcript
                                        .push(pi_tui::TranscriptItem::Status(notice));
                                } else {
                                    let notice =
                                        format!("OAuth login for {name} is not supported yet.");
                                    state
                                        .transcript
                                        .push(pi_tui::TranscriptItem::Status(notice));
                                }
                            }
                            None => {
                                login_panel = None;
                                state.close_panel();
                            }
                        }
                        dirty = true;
                        continue;
                    }
                    // 面板打开时，其它按键保持原样交给后续流程
                    _other => {}
                }
            }
            match apply(&mut state, event) {
                TuiAction::Exit => break 'main,
                TuiAction::Submit(text) => {
                    // `/login <provider>` 已选定 provider：这次提交的是 API key。
                    // ref `interactive-mode.ts:6003-6040`（api_key 型对话框）。
                    if let Some(provider) = state.pending_login_provider.take() {
                        let path = if state.agent_dir.as_os_str().is_empty() {
                            None
                        } else {
                            Some(state.agent_dir.join("auth.json"))
                        };
                        let key = text.trim().to_string();
                        let message = match (path, key.is_empty()) {
                            (_, true) => "API key must not be empty.".to_string(),
                            (None, false) => "Cannot write auth.json in this mode.".to_string(),
                            (Some(path), false) => {
                                match crate::auth_store::set_api_key(&path, &provider, &key) {
                                    // 成功提示按 ref 的措辞（不打印 key 本身）
                                    Ok(()) => {
                                        // ref `completeProviderAuthentication`：刷新并选中默认模型
                                        if let Some(model) =
                                            model_after_login(&state.agent_dir, &provider)
                                        {
                                            state.model = model;
                                        }
                                        // 登录后**总是**重建部件：`available_models`（`/model` 与
                                        // `Ctrl+P` 的数据源）是在装配时算的，只在 parts 为 None 时重建会导致
                                        // "已有模型时登录新 provider，`/model` 里看不到它的模型"（真机反馈）。
                                        if let Some(build) = rebuild.as_ref() {
                                            if let Some((built, model)) = build() {
                                                state.login_providers =
                                                    built.login_providers.clone();
                                                parts = Some(built);
                                                if !model.is_empty() {
                                                    state.model = model;
                                                }
                                            }
                                        }
                                        format!("Logged in to {provider}.")
                                    }
                                    Err(error) => format!("Login failed: {error}"),
                                }
                            }
                        };
                        state
                            .transcript
                            .push(pi_tui::TranscriptItem::Status(message));
                        dirty = true;
                        continue;
                    }
                    // P7-7f/perf：**回合串行化**。此前每次提交都无条件起一个新线程，
                    // 快速连发时会并发跑 N 个 agent 回合（各自持有完整上下文），
                    // 实测 30 轮快速输入 RSS 冲到 396 MB 并被 OOM 杀（退出码 137），
                    // 且多个回合的增量会交错写进同一 transcript（表现为内容错乱）。
                    if busy {
                        queued_inputs.push_back(text);
                        dirty = true;
                        continue;
                    }
                    busy = true;
                    // W3-2：用户消息进会话区（golden 形如 ` say hi`）；随后起一轮 agent run，
                    // 助手增量/工具调用/Working 由 `TuiEventSink` 经同一个 `tx` 送回本循环。
                    state.transcript.push(TranscriptItem::User(text.clone()));
                    if let Some(parts) = parts.as_ref() {
                        let parts = std::sync::Arc::clone(parts);
                        let tx = tx.clone();
                        let history = std::sync::Arc::clone(&history);
                        let level_snapshot = state.thinking_level.clone();
                        let turn_cancel_slot = std::sync::Arc::clone(&turn_cancel);
                        std::thread::spawn(move || {
                            let concrete_sink = std::sync::Arc::new(TuiEventSink::new(
                                tx.clone(),
                                context_window_of(&parts),
                                parts.system_message.clone(),
                            ));
                            let sink: std::sync::Arc<dyn pi_core::agent::EventSink> =
                                concrete_sink.clone();
                            // P7-7f M：每轮都用**当前生效**的模型（Ctrl+P / `/model` 切换后立即生效）。
                            let (provider_now, config_now) = parts
                                .model_state
                                .lock()
                                .map(|state| {
                                    let mut config = parts.config.clone();
                                    config.model.clone_from(&state.model_id);
                                    // 当前思考等级下发（ref `openai-completions.ts:925-928` /
                                    // `openai-responses-shared.ts` 的 `reasoning.effort`）。
                                    config.reasoning_effort =
                                        if parts.supports_thinking && level_snapshot != "off" {
                                            Some(level_snapshot.clone())
                                        } else {
                                            None
                                        };
                                    (std::sync::Arc::clone(&state.provider), config)
                                })
                                .unwrap_or_else(|_| {
                                    (std::sync::Arc::clone(&parts.provider), parts.config.clone())
                                });
                            let mut agent_loop = pi_core::agent::AgentLoop::new(
                                provider_now,
                                parts.tools.clone(),
                                sink,
                                config_now,
                            );
                            let user = pi_ai::message::Message::User {
                                content: pi_ai::UserContent::Text(text),
                                // 真实时间戳（毫秒）：会话层对拍要求与 TS 一致（truncate 用 0 会被
                                // compare-tui-layers 判不同；print 路径同样用真实时间）。
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as i64)
                                    .unwrap_or(0),
                                extra: serde_json::Map::new(),
                            };
                            // 落盘要跳过的前缀长度（=落盘前的历史条数；本轮的 user 不跳）。
                            let history_size = history.lock().map(|guard| guard.len()).unwrap_or(0);
                            // 本轮请求的上下文 = 历史（前几轮的 user/assistant）+ 本条 user。
                            let mut messages = history
                                .lock()
                                .map(|guard| guard.clone())
                                .unwrap_or_default();
                            messages.push(user);
                            // footer 百分比的口径：ref `compaction.ts:218-246 estimateContextTokens`
                            // 在没有 usage 时按**整个投影**估算（系统提示 + 历史 + 本轮输入）。
                            // 只算最后一条 assistant 会偏小（T10：我们 1.1% vs TS 1.2%）。
                            {
                                let mut projected = pi_core::agent::compaction::estimate_tokens(
                                    &parts.system_message,
                                );
                                for message in &messages {
                                    projected +=
                                        pi_core::agent::compaction::estimate_tokens(message);
                                }
                                concrete_sink.set_context_tokens(projected);
                            }
                            // 取消令牌随线程结束丢弃：TUI 退出时不阻塞在已无人接收的事件上。
                            let cancel = tokio_util::sync::CancellationToken::new();
                            if let Ok(mut slot) = turn_cancel_slot.lock() {
                                *slot = Some(cancel.clone());
                            }
                            // **只宣告本轮新消息**（ref `agent-loop.ts:209-216` 只宣告 pending）；
                            // 默认 `announce_from = 0` 会在每轮把**整段历史**重新宣告一遍，
                            // TUI 侧对每条 assistant 的 `message_end` 都会渲染一次错误提示 ⇒
                            // 第 k 轮屏幕上出现 k 份 `Error:`（真机"错误累计拼接"）。
                            agent_loop.set_announce_from(history_size);
                            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                            else {
                                return;
                            };
                            // **会话级 auto-retry**（ref `agent-session.ts:978-990` 的
                            // `_willRetryAfterAgentEnd` + `:1503` 的 `_prepareRetry`）：
                            // 本回合以"可重试错误"收尾且未到上限时，退避后**重跑整轮**。
                            // 默认上限 3 次、基础退避 2s（`settings.retry`）。
                            const RETRY_MAX: u32 = 3;
                            const RETRY_DELAY_MS: u64 = 2_000;
                            let mut attempt: u32 = 0;
                            let produced = loop {
                                agent_loop.set_retry_attempt(if attempt == 0 {
                                    None
                                } else {
                                    Some(attempt)
                                });
                                concrete_sink.set_retry_attempt(attempt);
                                let result = runtime.block_on(pi_core::agent::run::run_loop(
                                    &agent_loop,
                                    messages.clone(),
                                    &cancel,
                                ));
                                let Ok(produced) = result else {
                                    return;
                                };
                                let last_error =
                                    produced.last().and_then(|message| match message {
                                        pi_ai::message::Message::Assistant(assistant) => Some((
                                            assistant.stop_reason,
                                            assistant.error_message.clone(),
                                        )),
                                        _other => None,
                                    });
                                let retryable =
                                    last_error.as_ref().is_some_and(|(reason, error)| {
                                        pi_ai::ProviderError::is_retryable_message_text(
                                            reason,
                                            error.as_deref(),
                                        )
                                    });
                                if !retryable || attempt >= RETRY_MAX {
                                    // 只有"最后一次仍然可重试"才是真失败；重试成功后 `attempt>0`
                                    // 但 `retryable=false`，不能报 `Retry failed`（真机表现为成功
                                    // 之后仍显示重试失败）。
                                    let gave_up = attempt > 0 && retryable;
                                    if gave_up {
                                        // 退避状态行是**瞬时**的（ref `RetryStatusIndicator`）：
                                        // 先移除，再打最终失败文案，否则屏幕上会留着
                                        // `Retrying (3/3) in 1s...` 一行（真机看到的就是这个）。
                                        let _ = tx.send(TuiEvent::ClearStatus);
                                        // 最后一次仍失败：ref 的 `showError`
                                        // `Retry failed after N attempts: <err>`
                                        // （`interactive-mode.ts:3631`）。
                                        let detail = last_error
                                            .as_ref()
                                            .and_then(|(_reason, error)| error.clone())
                                            .unwrap_or_else(|| String::from("Unknown error"));
                                        let _ = tx.send(TuiEvent::Status(format!(
                                            "Retry failed after {attempt} attempts: {detail}"
                                        )));
                                    }
                                    if !gave_up {
                                        // 成功/取消：状态行是瞬时的，移掉。
                                        let _ = tx.send(TuiEvent::ClearStatus);
                                    }
                                    break produced;
                                }
                                attempt += 1;
                                // **指数退避**：TS 实测第 1/2/3 次分别是 2s / 4s / 8s
                                // （`agent-session.ts:1503` 的 `_prepareRetry` 每次翻倍）。
                                let delay_ms =
                                    RETRY_DELAY_MS.saturating_mul(1_u64 << (attempt - 1));
                                // ref `status-indicator.ts:54-63` 的倒计时文案。
                                let _ = tx.send(TuiEvent::Status(format!(
                                    "Retrying ({attempt}/{RETRY_MAX}) in {}s... (escape to cancel)",
                                    delay_ms / 1000
                                )));
                                // 分片睡眠：Escape 取消（ref 重试期同样响应 interrupt），
                                // 同时**每秒刷新**倒计时文案（ref `status-indicator.ts:54-63`：
                                // `Retrying (n/3) in Xs... (escape to cancel)`，X 逐秒递减）。
                                let mut waited = 0_u64;
                                let mut shown_seconds = delay_ms / 1000;
                                while waited < delay_ms {
                                    if cancel.is_cancelled() {
                                        break;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                    waited += 50;
                                    let left = delay_ms.saturating_sub(waited);
                                    // ref 的倒计时**到 1 就执行**（不出现 `in 0s`）。
                                    let seconds = left.div_ceil(1000).max(1);
                                    if seconds != shown_seconds {
                                        shown_seconds = seconds;
                                        let _ = tx.send(TuiEvent::Status(format!(
                                            "Retrying ({attempt}/{RETRY_MAX}) in {seconds}s... (escape to cancel)"
                                        )));
                                    }
                                }
                                if cancel.is_cancelled() {
                                    // 取消重试：状态行同样是瞬时的。
                                    let _ = tx.send(TuiEvent::ClearStatus);
                                    break produced;
                                }
                            };

                            // `run_loop` 返回的是**整段上下文**（含我们传进去的初始消息）⇒ 落盘时只取
                            // 超出初始长度的尾部，否则第二轮会把第一轮的消息再写一遍（T08 实测 pi-rs 10 条
                            // 对 TS 8 条，多出的正是重复的 user/assistant 对）。
                            {
                                // 会话落盘：**按 print 路径已验证的规则**（`print_mode.rs:230-243`）——
                                // 只写"初始上下文之后的新产出"，但跳过量取 **`history.len()`**（不是整段
                                // `messages.len()`）：TUI 把本轮的 user 也放在初始上下文里，而 print 路径的
                                // user 是循环自己追加的；跳过整段会连本轮 user 一起丢掉（实测只剩 6 条）。
                                let to_write_from = history_size;
                                // 会话落盘：按 `session_writer` 既有顺序逐条追加（与 print 路径同一个 API，
                                // `session_writer.rs:325 append_message`），不在这里重写格式。
                                if let Some(writer) = parts.writer.as_ref() {
                                    if let Ok(mut handle) = writer.lock() {
                                        for message in
                                            produced.iter().skip(to_write_from.min(produced.len()))
                                        {
                                            // provider 回来的消息可能没带时间戳（pi-rs 侧为 0）；TS 的会话里
                                            // 是真实毫秒时间（会话层对拍要求一致）⇒ 落盘前补当前时间。
                                            let mut stamped = message.clone();
                                            stamp_timestamp(&mut stamped);
                                            if let Err(error) = handle.append_message(&stamped) {
                                                eprintln!("pi-rs: 写会话失败：{error}");
                                            }
                                        }
                                    }
                                }
                                // 把本轮**新产出**并入历史（供下一轮请求体）。
                                // `produced` 是整段上下文（前 `history_size` 条是我们传进去的历史），
                                // 全量并入会让历史每轮**翻倍**：真机表现就是"越往后轮次 Working
                                // 等得越久"（离线实测请求体 5.8k→33MB、第 20 轮 21 秒）。
                                if let Ok(mut guard) = history.lock() {
                                    guard.extend(
                                        produced_tail(history_size, &produced).iter().cloned(),
                                    );
                                }
                                messages.extend(produced);
                            }
                            // footer 用量：percent 照 ref `agent-session.ts:3893-3895` 的口径
                            // （`estimateProjectedContextTokens(...).tokens / contextWindow × 100`）用
                            // `estimate_context_tokens` 近似；格式照 `footer.ts:150-155`。
                            let estimate_messages: Vec<pi_ai::message::Message> =
                                std::iter::once(parts.system_message.clone())
                                    .chain(messages.iter().cloned())
                                    .collect();
                            let estimate = pi_core::agent::compaction::estimate_context_tokens(
                                &estimate_messages,
                            );
                            let (input, output) = messages
                                .iter()
                                .rev()
                                .find_map(|message| {
                                    // 不用 match 的通配分支（lint-banned 禁止对核心枚举用 ）。
                                    if let pi_ai::message::Message::Assistant(assistant) = message {
                                        Some((assistant.usage.input, assistant.usage.output))
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or((0, 0));
                            let window_tokens = context_window_of(&parts);
                            let window = if window_tokens >= 1000 {
                                format!("{}k", window_tokens / 1000)
                            } else {
                                window_tokens.to_string()
                            };
                            let percent = if window_tokens == 0 {
                                None
                            } else {
                                Some(estimate.tokens as f64 / window_tokens as f64 * 100.0)
                            };
                            // 回合结束时只更新**百分比**（`estimate` 是整段上下文的投影口径，
                            // ref `footer.ts:150-155`）；累计用量已由本轮 `message_end` 累加，
                            // 这里再传一次会双计 ⇒ input/output 传 0。
                            let _ = input;
                            let _ = output;
                            // footer 的 `↑n ↓n $x` 取**整个会话条目**的求和（ref `footer.ts:86-105`）：
                            // 事件累加会在压缩/续写后与 TS 不一致，也会让数字"变小"。
                            let (total_in, total_out, total_cost) = match parts.writer.as_ref() {
                                Some(writer) => writer.lock().map_or((0, 0, 0.0), |guard| {
                                    crate::session_writer::session_usage_totals(guard.path())
                                }),
                                None => (0, 0, 0.0),
                            };
                            let _ = tx.send(TuiEvent::UsageTotals {
                                input: total_in,
                                output: total_out,
                                cost: total_cost,
                                percent,
                                window,
                            });
                            // 回合结束：显式撤掉  行。**不能依赖 **（P7-4d 实测它不会到达 sink）。
                            let _ = tx.send(TuiEvent::Working(false));
                        });
                    }
                    dirty = true;
                }
                TuiAction::Command(name) => {
                    // 已知内建命令（P7-6/W2）：本批只实现 `/quit`、`/session`、`/hotkeys`、`/settings`、`/model`。
                    // 命令名只取首词，参数留在 `name` 里（ref `/login <provider>` :3172）
                    let command_word = name.split_whitespace().next().unwrap_or_default();
                    match command_word {
                        "quit" => break,
                        "session" => {
                            // `/session` 面板（golden-tui/T04 实测形状与字段顺序）：文件/ID/消息统计/token/Cache Warming。
                            let path = parts
                                .as_ref()
                                .and_then(|parts| parts.writer.as_ref())
                                .and_then(|writer| {
                                    writer
                                        .lock()
                                        .ok()
                                        .map(|handle| handle.path().display().to_string())
                                })
                                .unwrap_or_else(|| "(未写会话)".to_string());
                            let id = parts
                                .as_ref()
                                .and_then(|parts| parts.writer.as_ref())
                                .and_then(|writer| {
                                    writer.lock().ok().and_then(|handle| handle.header_id())
                                })
                                .unwrap_or_else(|| "(no-id)".to_string());
                            let panel = vec![
                                " Session Info".to_string(),
                                String::new(),
                                format!(" File: {path}"),
                                format!(" ID: {id}"),
                                String::new(),
                                " Messages".to_string(),
                                " Total: 0".to_string(),
                                " User: 0".to_string(),
                                " Assistant: 0".to_string(),
                                " Tools: 0 calls, 0 results".to_string(),
                                String::new(),
                                " Tokens".to_string(),
                                " Input: 0".to_string(),
                                " Output: 0".to_string(),
                                " Total: 0".to_string(),
                                String::new(),
                                " Cache Warming".to_string(),
                                " Mode: streaming".to_string(),
                                " Status: Inactive (waiting for first request)".to_string(),
                            ];
                            state.open_panel(pi_tui::PanelPlacement::Transcript, panel);
                        }
                        "hotkeys" => {
                            // `/hotkeys`：多段快捷键表格（数据来自 golden 的逐字转录；画在会话区）。
                            let lines: Vec<String> = pi_tui::HOTKEYS_LINES
                                .iter()
                                .map(|line| (*line).to_string())
                                .collect();
                            state.open_panel(pi_tui::PanelPlacement::Transcript, lines);
                        }
                        "login" => {
                            // ref `interactive-mode.ts:3171-3175`：`/login [provider]`。带
                            // providerRef 且命中唯一 api_key 型 provider 时直接进 key 输入态
                            // （ref `startProviderLogin` :5708-5716），否则弹 provider 面板。
                            let provider_ref = name
                                .split_once(' ')
                                .map(|(_, rest)| rest.trim().to_string())
                                .filter(|value| !value.is_empty());
                            let options =
                                crate::login_options::login_options(&state.login_providers);
                            let matched: Vec<_> = match provider_ref.as_ref() {
                                Some(provider_ref) => options
                                    .iter()
                                    .filter(|option| {
                                        option.id.eq_ignore_ascii_case(provider_ref)
                                            || option.name.eq_ignore_ascii_case(provider_ref)
                                    })
                                    .collect(),
                                None => Vec::new(),
                            };
                            // 同一 provider 同时有 oauth 与 api_key 时（如 anthropic），
                            // 优先走已实现的 api_key（ref 会先问认证方式；OAuth 尚未实现）。
                            let matched_api_key: Vec<_> = matched
                                .iter()
                                .copied()
                                .filter(|option| {
                                    matches!(
                                        option.auth_type,
                                        crate::login_options::AuthType::ApiKey
                                    )
                                })
                                .collect();
                            let matched = if matched_api_key.is_empty() {
                                matched
                            } else {
                                matched_api_key
                            };
                            match (provider_ref.as_deref(), matched.first()) {
                                (Some(_), Some(option))
                                    if matches!(
                                        option.auth_type,
                                        crate::login_options::AuthType::ApiKey
                                    ) =>
                                {
                                    let id = option.id.clone();
                                    let notice =
                                        format!("Logging in to {} (api_key).", option.name);
                                    state.pending_login_provider = Some(id);
                                    state.input.clear();
                                    state.cursor = 0;
                                    state
                                        .transcript
                                        .push(pi_tui::TranscriptItem::Status(notice));
                                }
                                (Some(_), Some(option)) => {
                                    // OAuth 型本版本不支持（LIMITATIONS.md）
                                    let notice = format!(
                                        "OAuth login for {} is not supported yet.",
                                        option.name
                                    );
                                    state
                                        .transcript
                                        .push(pi_tui::TranscriptItem::Status(notice));
                                }
                                (Some(provider_ref), None) => {
                                    let notice = format!("No provider matches \"{provider_ref}\".");
                                    state
                                        .transcript
                                        .push(pi_tui::TranscriptItem::Status(notice));
                                }
                                (None, _) => {
                                    {
                                        // ref 即使没有任何 provider 也先给认证方式选择
                                        let items = login_method_items();
                                        login_render(&mut state, &items, 0);
                                        login_panel = Some((items, 0));
                                    }
                                }
                            }
                        }
                        "logout" => {
                            // ref `interactive-mode.ts:3177-3179`：`/logout` 删已存凭据。
                            let path = parts
                                .as_ref()
                                .map(|parts| parts.agent_dir.join("auth.json"));
                            let mut removed = Vec::new();
                            if let Some(path) = path {
                                for provider in &state.login_providers {
                                    if crate::auth_store::remove_provider(&path, provider)
                                        .unwrap_or(false)
                                    {
                                        removed.push(provider.clone());
                                    }
                                }
                            }
                            let text = if removed.is_empty() {
                                // ref `interactive-mode.ts:5834` 的文案
                                "No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json are unaffected."
                                    .to_string()
                            } else {
                                format!("Logged out of {}.", removed.join(", "))
                            };
                            state.transcript.push(pi_tui::TranscriptItem::Status(text));
                        }
                        "settings" => {
                            // P7-7c1：`/settings` 主列表（条目顺序与两列渲染见 `settings_selector`）。
                            let mut sel = crate::settings_selector::SettingsSelector::new();
                            if let Some(parts) = parts.as_ref() {
                                sel.last_model_id = Some(parts.config.model.clone());
                                sel.last_provider_id = Some(parts.config.provider.clone());
                            }
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(cols));
                            selector = Some(SelectorUi::Settings(sel, cols));
                        }
                        "model" => {
                            // `/model` 选择器（数据来自 golden 的可见行；画在编辑器下方）。
                            // P7-7a：真实选择器。列表来自装配时的模型（当前只有一个），
                            // 刷新状态文案按 ref `:210` 成功分支。
                            // 列表来自 `models.json` 里该 provider 的全部模型；
                            // 当前模型打 `✓`，其余可上下移动后回车切换（ref `model-selector.ts:316-330`）。
                            // 一条模型都没有（既没 `models.json` 也没可用的内置目录）：
                            // ref `core/model-resolver.ts:422-427` 只报错、不造条目；
                            // 早先这里塞了硬编码的 `mock-model`，真机上会显示成
                            // "✓ mock-model [openai-completions] · default"（假模型）。
                            if parts.is_none() {
                                state.transcript.push(pi_tui::TranscriptItem::Status(
                                    "No models available. Check your installation or add models to models.json."
                                        .to_string(),
                                ));
                                dirty = true;
                                continue;
                            }
                            let (choices, current_id, current_provider) =
                                parts.as_ref().map_or_else(
                                    || (Vec::new(), String::new(), String::new()),
                                    |parts| {
                                        (
                                            parts.available_models.clone(),
                                            parts
                                                .model_state
                                                .lock()
                                                .map(|state| state.model_id.clone())
                                                .unwrap_or_else(|_| parts.config.model.clone()),
                                            parts
                                                .model_state
                                                .lock()
                                                .map(|state| state.provider_name.clone())
                                                .unwrap_or_else(|_| parts.config.provider.clone()),
                                        )
                                    },
                                );
                            let entries: Vec<crate::model_selector::ModelEntry> = choices
                                .iter()
                                .map(|choice| crate::model_selector::ModelEntry {
                                    id: choice.id.clone(),
                                    provider: choice.provider.clone(),
                                    name: choice.id.clone(),
                                    current: choice.id == current_id
                                        && choice.provider == current_provider,
                                    default: choice.id == current_id
                                        && choice.provider == current_provider,
                                })
                                .collect();
                            let model_sel = crate::model_selector::ModelSelector::new(
                                entries,
                                Some("Model catalogs refreshed.".to_string()),
                            );
                            state
                                .open_panel(pi_tui::PanelPlacement::BelowEditor, model_sel.lines());
                            selector = Some(SelectorUi::Model(model_sel));
                        }
                        "tree" => {
                            // P7-7e1/e2：会话树（布局 ref `tree-selector.ts:669-770` + 头部帮助 `:1186-1220`）。
                            let tree_rows = parts.as_ref().map_or_else(Vec::new, |parts| {
                                parts
                                    .writer
                                    .as_ref()
                                    .and_then(|writer| {
                                        let path = writer.lock().ok()?.path().to_path_buf();
                                        crate::settings_selector::tree_rows_from_session(&path).ok()
                                    })
                                    .unwrap_or_default()
                            });
                            let mut sel = crate::settings_selector::TreeSelector::new(tree_rows);
                            // ref 的 `maxVisibleLines`（T61 实测 = rows/2 = 15）：既是窗口高度也是翻页步进。
                            sel.max_visible = (rows / 2).max(1);
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(cols));
                            selector = Some(SelectorUi::Tree(sel, cols));
                        }
                        "resume" => {
                            // P7-7d1/d3：会话选择器（布局 ref `session-selector.ts:742-755`）。
                            // 会话来自配置目录旁的 session 目录；`Current Folder` 按 cwd 过滤。
                            let mut sel = crate::settings_selector::ResumeSelector::new();
                            if let Some(parts) = parts.as_ref() {
                                sel.sessions =
                                    crate::settings_selector::scan_sessions(&parts.session_dir);
                                sel.current_cwd = parts.cwd.clone();
                                // ref `session-manager.ts:1904-1906`：默认分区目录里**不**按
                                // header cwd 过滤（目录改名后旧会话仍要能选）；自定义目录才过滤。
                                sel.filter_cwd = crate::session_writer::should_filter_by_cwd(
                                    &parts.session_dir,
                                    std::path::Path::new(&parts.cwd),
                                    &parts.agent_dir,
                                );
                            }
                            sel.home = crate::runtime::home_from(&|key| std::env::var(key).ok());
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines(cols));
                            selector = Some(SelectorUi::Resume(sel, cols));
                        }
                        "thinking" => {
                            // P7-7b：`/thinking` 选择器（ref `thinking-selector.ts:74-104`）。
                            // 档位集合来自当前模型能力：非推理模型只有 `off`，推理模型是七档
                            // （ref `ai/src/models.ts:924-933`）；` · default` 标记来自
                            // settings 的 `defaultThinkingLevel`（ref `interactive-mode.ts:5012-5025`）。
                            let default_level = parts.as_ref().and_then(|parts_ref| {
                                std::fs::read_to_string(parts_ref.agent_dir.join("settings.json"))
                                    .ok()
                                    .and_then(|text| crate::settings::parse(&text).ok())
                                    .and_then(|settings| settings.default_thinking_level)
                            });
                            let levels = state.available_thinking_levels.clone();
                            let sel = crate::thinking_selector::ThinkingSelector::new(
                                levels
                                    .iter()
                                    .map(|level| crate::thinking_selector::ThinkingEntry {
                                        level: level.clone(),
                                        current: *level == state.thinking_level,
                                        default: default_level.as_deref() == Some(level.as_str()),
                                    })
                                    .collect(),
                            );
                            state.open_panel(pi_tui::PanelPlacement::BelowEditor, sel.lines());
                            selector = Some(SelectorUi::Thinking(sel));
                        }
                        other => {
                            // 其余命令尚未实现 ⇒ 不改变屏幕。
                            let _ = other;
                        }
                    }
                    dirty = true;
                }
                TuiAction::Redraw => dirty = true,
            }
        }
        if let Ok((c, r)) = terminal::size() {
            if c as usize != cols || r as usize != rows {
                cols = c as usize;
                rows = r as usize;
                previous.clear();
                dirty = true;
            }
        }
    }
    let _ = draw(
        &state,
        cols,
        rows,
        &mut previous,
        &mut diff_state,
        &mut render_cache,
    );
    // P7-6o(续3)/W2.4：r3 退出前补 `beforeTerminalStop`（`tui-main-screen.ts:168-176`）——
    // 把光标移到内容末尾下方并换行；该序列让主屏多滚一行，正是 golden 全文比内容多一行的来源。
    {
        let mut out = std::io::stdout();
        // P7-6p/W2.3：改用 `diff::before_terminal_stop`（与 `fixtures-exit` 逐字节一致）。
        let stop = pi_tui::diff::before_terminal_stop(&diff_state);
        let _ = out.write_all(stop.as_bytes());
        let _ = out.flush();
    }
    guard.restore();
    // 退出后打印**恢复会话**提示（ref 退出路径同款文案，见 T01 golden 尾部）：
    // `\x1b[2mTo resume this session:\x1b[22m pi --session-dir <dir> --session <id>`。
    // 之前完全没有这行，样式级对拍里那 23 格就是它。
    if let Some(parts) = parts.as_ref() {
        if let Some(id) = session_id_for_resume(parts).filter(|_| session_is_flushed(parts)) {
            let mut out = std::io::stdout();
            // ref `interactive-mode.ts:4173` 用的是 `process.stdout.write("…\n")`——
            // **只发 LF**。`writeln!` 在 Windows 上会发 CRLF，退出后会多出一行/多一个
            // 回车（真机看到的"退出后多一个空行"），所以这里显式写 `\n`。
            let _ = write!(
                out,
                "\u{1b}[2mTo resume this session:\u{1b}[22m pi --session-dir {} --session {id}",
                parts.session_dir.display()
            );
            // 只发 LF（不用 `write!` 的换行简写，避免 clippy 的 write_with_newline）。
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
    }
    Ok(0)
}

/// 会话是否已落盘（ref `formatResumeCommand` 要求 `existsSync(sessionFile)`；
/// 纯 UI 场景没有任何条目 ⇒ 没有文件 ⇒ 不打印退出提示）。
fn session_is_flushed(parts: &AgentParts) -> bool {
    parts
        .writer
        .as_ref()
        .and_then(|writer| writer.lock().ok())
        .map(|guard| guard.is_flushed())
        .unwrap_or(false)
}

/// 取本次会话的 id（用于退出提示）。读最后一条 `session` 头里的 `id`。
fn session_id_for_resume(parts: &AgentParts) -> Option<String> {
    let writer = parts.writer.as_ref()?;
    let guard = writer.lock().ok()?;
    guard.header_id()
}

/// `PI_TUI_OUTPUT_DUMP` 对应的调试文件（未设置 ⇒ `None`，不建文件、不格式化）。
///
/// 值为 `1` 时写到系统临时目录的 `pi-rs-output.log`，否则当作路径。只解析一次。
fn output_dump_path() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let value = std::env::var("PI_TUI_OUTPUT_DUMP").ok()?;
        if value.is_empty() {
            return None;
        }
        Some(if value == "1" {
            std::env::temp_dir().join("pi-rs-output.log")
        } else {
            std::path::PathBuf::from(value)
        })
    })
    .clone()
}

/// 把一帧**写给终端的原始字节**落盘（`PI_TUI_OUTPUT_DUMP`）。
///
/// 用途：终端上多出 `1~` / `;;` 这类字符时，只有输出侧原文能区分"我们发错了"
/// 还是"PTY/终端解析坏了"。转义成一行一条终端行，`\e` = ESC。
fn dump_output(
    path: &std::path::Path,
    cols: usize,
    rows: usize,
    diff_state: &pi_tui::diff::DiffState,
    payload: &str,
) {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let index = SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(
        file,
        "=== frame {index} size={cols}x{rows} viewport_top={} hw_row={} bytes={} ===",
        diff_state.previous_viewport_top,
        diff_state.hardware_cursor_row,
        payload.len()
    );
    let mut escaped = String::with_capacity(payload.len());
    for ch in payload.chars() {
        match ch {
            '\u{1b}' => escaped.push_str("\\e"),
            '\r' => escaped.push_str("\\r"),
            '\n' => escaped.push_str("\\n\n"),
            '\u{7}' => escaped.push_str("\\a"),
            control if (control as u32) < 0x20 => {
                escaped.push_str(&format!("\\x{:02x}", control as u32));
            }
            other => escaped.push(other),
        }
    }
    let _ = writeln!(file, "{escaped}");
}

/// `PI_TUI_CURSOR_DUMP` 对应的调试文件（未设置 ⇒ `None`，不建文件、不格式化）。
///
/// 值为 `1` 时写到系统临时目录的 `pi-rs-cursor.log`，否则当作路径。
fn cursor_dump_file() -> Option<std::fs::File> {
    let value = std::env::var("PI_TUI_CURSOR_DUMP").ok()?;
    if value.is_empty() {
        return None;
    }
    let path = if value == "1" {
        std::env::temp_dir().join("pi-rs-cursor.log")
    } else {
        std::path::PathBuf::from(value)
    };
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

/// 一帧的光标转储行（纯函数，便于单测；字段与 V6 的对比口径一致）。
fn format_cursor_line(
    frame: u64,
    elapsed_ms: u64,
    cols: usize,
    rows: usize,
    diff: &pi_tui::diff::DiffState,
    state: &TuiState,
) -> String {
    format!(
        "frame={frame} ms={elapsed_ms} size={cols}x{rows} hw_row={} col={} input={} cursor={} busy={} thinking={}",
        diff.hardware_cursor_row,
        diff.cursor_col,
        state.input.chars().count(),
        state.cursor,
        state.working,
        state.thinking_level
    )
}

/// 追加一帧（I/O 失败一律忽略：调试输出不能影响 TUI 主循环）。
fn write_cursor_dump(
    file: &mut std::fs::File,
    frame: u64,
    elapsed_ms: u64,
    cols: usize,
    rows: usize,
    diff: &pi_tui::diff::DiffState,
    state: &TuiState,
) {
    use std::io::Write as _;
    let line = format_cursor_line(frame, elapsed_ms, cols, rows, diff, state);
    let _ = writeln!(file, "{line}");
    let _ = file.flush();
}

/// `showHardwareCursor` 的取值（ref `core/settings-manager.ts:1369-1370`）：
/// settings 优先，其次 `PI_HARDWARE_CURSOR=1`，都没有则默认关（`tui.ts:478`）。
fn resolve_show_hardware_cursor(settings_value: Option<bool>, env_value: Option<&str>) -> bool {
    settings_value.unwrap_or_else(|| env_value == Some("1"))
}

/// 把当前状态画到终端（光标回左上角逐行覆盖，避免整屏闪烁）。
fn draw(
    state: &TuiState,
    cols: usize,
    rows: usize,
    _previous: &mut Vec<String>,
    diff_state: &mut pi_tui::diff::DiffState,
    render_cache: &mut pi_tui::RenderCache,
) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    let mut buffer = String::new();
    // P8e-18：帧内耗时（`PI_TUI_TIMING=<path>`，仅排查大帧用）。
    let timing = std::env::var_os("PI_TUI_TIMING");
    let started = std::time::Instant::now();
    let build_ms = std::cell::Cell::new(0_u128);
    {
        {
            // P7-6n：主屏差分渲染（`TuiMainScreen.doRender` 移植）。`render_all` 给**全量**行，
            // 可见窗口由 `diff_render` 的 `height` 决定（不预裁剪）。
            // **不再外包同步序列**：`diff_render` 自己已经写出 `?2026h/…/?2026l`（`tui-main-screen.ts:283/317/494/567`）。
            let frame = pi_tui::render_all_marker_cached_styled(state, cols, rows, render_cache);
            // S1：样式/坐标诊断转储（`PI_TUI_STYLE_DUMP=<path>`，仅排查用）。
            if let Some(path) = std::env::var_os("PI_TUI_STYLE_DUMP") {
                use std::io::Write as _;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    let _ = writeln!(
                        file,
                        "=== frame viewport_top={} lines={} cursor_row={} ===",
                        diff_state.previous_viewport_top,
                        frame.lines.len(),
                        diff_state.cursor_row
                    );
                    for (index, line) in frame.lines.iter().enumerate() {
                        let style = frame.styles.get(index).copied().flatten();
                        let span_count = frame.spans.get(index).map_or(0, Vec::len);
                        let _ = writeln!(
                            file,
                            "{index}\tfg={:?}\tbg={:?}\tdim={}\tspans={span_count}\t{:?}",
                            style.and_then(|s| s.fg),
                            style.and_then(|s| s.bg),
                            style.is_some_and(|s| s.dim),
                            line.replace('\u{1b}', "\\e")
                        );
                    }
                }
            }
            build_ms.set(started.elapsed().as_millis());
            let lines = frame.lines;
            let styles = frame.styles;
            let spans = frame.spans;
            // W1：诊断转储（feature 关闭时这段不参与编译）。
            #[cfg(feature = "diag-frame-dump")]
            {
                static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                diag_dump::dump_frame(seq, cols, rows, &lines, diff_state, 0);
            }
            // 每帧把"是否有浮层"告知差分层（ref `hasOverlayEntries`）。
            diff_state.has_overlay = state.panel.is_some();
            match pi_tui::diff::diff_render_with_cursor_styled(
                diff_state, &lines, &styles, &spans, cols, rows,
            ) {
                Ok((render_bytes, cursor_bytes)) => {
                    // 顺序同 ref：渲染字节 → 光标字节（`positionHardwareCursor` 在同步区之后写出）。
                    buffer.push_str(&render_bytes);
                    buffer.push_str(&cursor_bytes);
                }
                Err(error) => {
                    // ref 在此处写 crash log 后 `throw`（`tui-main-screen.ts:534-548`）；
                    // r3 的处置见 `notes/reports/P7-6n-wiring.md`：不 panic，跳帧并报一次 stderr。
                    eprintln!("pi-rs[r3]: 渲染错误，跳过本帧：{error}");
                }
            }
        }
    }
    // W3：与 TS 一致，把整屏重绘包在**同步更新**序列里（`\x1b[?2026h` … `\x1b[?2026l`）。
    // 作用：① 终端不会显示半张画面；② 录制的原始流因此有**帧边界**，比较时才能把截断点
    // 回退到完整帧的结尾（见 notes/reports/P7-4c-trim-diag.md 与 notes/decisions.md）。
    let payload = buffer; // r3：`diff_render` 已自带同步区，不能再包一层
    let build_ms = build_ms.get();
    let diff_ms = started.elapsed().as_millis();
    if let Some(path) = output_dump_path() {
        dump_output(&path, cols, rows, diff_state, &payload);
    }
    let result = write_frame(&mut out, &payload);
    if let Some(path) = timing {
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(
                file,
                "frame lines={} bytes={} build={build_ms}ms diff={}ms write={}ms",
                payload.lines().count(),
                payload.len(),
                diff_ms as i128 - build_ms as i128,
                started.elapsed().as_millis() as i128 - diff_ms as i128
            );
        }
    }
    result
}

/// 把一帧字节按**行边界**分块写给终端（切点只在 `\n` 之后）。
///
/// 约束：
/// 1. 切点只落在 `\n` 之后 —— 单条转义序列不会被写入层切开（行尾序列被打坏的真实原因在
///    渲染层，见 `diff::shift_past_marker`）。
/// 2. 分块上限 64 KiB：Windows 控制台句柄单次写入有长度上限，超过会被截断；
///    Linux 侧顺带把"每行一次 `write_all`"降成每 64 KiB 一次。
///
/// 为什么改成分块：69 MB / 34.5 万行的会话整屏重绘一次是 35 MB 输出，逐行写等于
/// 36.5 万次 `write` 系统调用（`/proc/<pid>/io` 的 `syscw` 实测就是这个数），
/// `Ctrl+P` 一次要 7 秒；分块后同样字节量只需要几百次写入。字节内容与原实现**逐字节相同**。
///
/// 排查开关：`PI_TUI_OUTPUT_DUMP` 会把每帧实际写出的字节落盘（见 `dump_output`）。
const WRITE_CHUNK: usize = 64 * 1024;

fn write_frame(out: &mut impl std::io::Write, payload: &str) -> std::io::Result<()> {
    let bytes = payload.as_bytes();
    let mut start = 0_usize;
    // 当前块内最后一个换行之后的位置（切点候选）。
    let mut cut = 0_usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            cut = index + 1;
        }
        if index + 1 - start >= WRITE_CHUNK && cut > start {
            out.write_all(&bytes[start..cut])?;
            start = cut;
        }
    }
    if start < bytes.len() {
        out.write_all(&bytes[start..])?;
    }
    out.flush()
}

/// 把 agent loop 的事件接成 TUI 的语义事件（W3-2）。
///
/// 映射（只挑会话区/footer 需要的）：
/// - `MessageUpdate{assistant_event: Some(TextDelta{delta,..})}` ⇒ `TuiEvent::AssistantDelta`（流式增量；
///   `apply` 会把连续增量合并进**同一条**助手消息）；
/// - `ToolExecutionStart{tool_name,args}` ⇒ `TuiEvent::ToolCall{name,summary}`（summary 取参数里第一个字符串值，
///   照 golden/T06 实测的 `read input.txt` 形态）；
/// - `AgentStart` ⇒ `Working(true)`；`AgentSettled` ⇒ `Working(false)`。
///
/// **线程安全**：只持有一个 `Sender`，可跨线程共享（`Send + Sync`）。
/// **取消/退出**：发送失败（TUI 已退出、接收端关闭）时**丢弃事件**，不 panic、不阻塞。
pub struct TuiEventSink {
    tx: std::sync::mpsc::Sender<TuiEvent>,
    /// 上下文窗口 token 数（footer 的 `x%/Nk` 用它换算；来自模型配置）。
    context_window: u64,
    /// 会话里 system 条目的原始形态（footer 百分比估算用；见 `AgentParts.system_message`）。
    system_message: pi_ai::message::Message,
    /// 当前是第几次重试（0 = 首次尝试）。>0 时**不宣告中间失败**——
    /// ref 只在最终放弃时才把错误留在会话区，否则每次重试都会多一个 `Error:` 块。
    retry_attempt: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// 本轮投影的 token 估算（系统提示 + 全部消息）；0 = 未设置。
    ///
    /// ref `compaction.ts:218-246` 的 `estimateContextTokens`：没有 usage 时百分比按**整个投影**算，
    /// 只算最后一条 assistant 会偏小（T10 实测 1.1% vs TS 1.2%）。
    context_tokens: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl TuiEventSink {
    /// 标记当前是第几次重试（0 = 首次）。
    pub fn set_retry_attempt(&self, attempt: u32) {
        self.retry_attempt
            .store(attempt, std::sync::atomic::Ordering::Relaxed);
    }

    /// 用事件循环的发送端构造。
    #[must_use]
    pub fn new(
        tx: std::sync::mpsc::Sender<TuiEvent>,
        context_window: u64,
        system_message: pi_ai::message::Message,
    ) -> Self {
        Self {
            tx,
            context_window,
            system_message,
            retry_attempt: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
            context_tokens: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// 记下本轮投影的 token 估算（系统提示 + 历史 + 本轮输入），供 footer 百分比使用。
    pub fn set_context_tokens(&self, tokens: u64) {
        self.context_tokens
            .store(tokens, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 落盘前给"时间戳为 0"的消息补上当前毫秒时间（TS 的会话里每条消息都带真实时间）。
fn stamp_timestamp(message: &mut pi_ai::message::Message) {
    if message.timestamp() != 0 {
        return;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    match message {
        pi_ai::message::Message::System { timestamp, .. }
        | pi_ai::message::Message::User { timestamp, .. } => *timestamp = now_ms,
        pi_ai::message::Message::Assistant(assistant) => assistant.timestamp = now_ms,
        pi_ai::message::Message::ToolResult { timestamp, .. }
        | pi_ai::message::Message::Custom { timestamp, .. }
        | pi_ai::message::Message::BranchSummary { timestamp, .. }
        | pi_ai::message::Message::CompactionSummary { timestamp, .. } => *timestamp = now_ms,
    }
}

/// 从工具参数 JSON 里取一个可读摘要：优先第一个字符串值，否则紧凑 JSON（空对象则空串）。
fn tool_summary(args: &serde_json::Value) -> String {
    if let Some(object) = args.as_object() {
        for value in object.values() {
            if let Some(text) = value.as_str() {
                return text.to_string();
            }
        }
    }
    match args {
        serde_json::Value::Null => String::new(),
        other if other.as_object().map(|o| o.is_empty()).unwrap_or(false) => String::new(),
        other => other.to_string(),
    }
}

impl pi_core::agent::EventSink for TuiEventSink {
    fn emit(&self, event: pi_core::agent::AgentEvent) {
        use pi_core::agent::AgentEvent;
        let mapped = match event {
            AgentEvent::MessageEnd { message } => match message {
                // 助手消息定稿 ⇒ 更新 footer 左段（照 footer.ts:132-135/150-155）。
                pi_ai::message::Message::Assistant(assistant) => {
                    // P7-6an/P（真机 5b「问话后无反馈」）：provider 失败在 ref 里是**一条
                    // `stopReason:"error"` 的 assistant 消息**，由 `assistant-message.ts:182-199`
                    // 渲染成 `Error: <errorMessage>`；`length` 渲染"Response was truncated…"；
                    // `aborted` 渲染 errorMessage 或 "Operation aborted"。
                    // 我们原来只渲染 `content`（错误消息 content 为空）⇒ 屏幕上一片空白。
                    let notice = crate::tui::assistant_notice(
                        &assistant.stop_reason,
                        assistant.error_message.as_deref(),
                    );
                    let notice_delta = notice;
                    let window = if self.context_window >= 1000 {
                        format!("{}k", self.context_window / 1000)
                    } else {
                        self.context_window.to_string()
                    };
                    let percent = if self.context_window == 0 {
                        None
                    } else if pi_ai::calculate_context_tokens(&assistant.usage) > 0 {
                        // 有 usage（T01 的 `↑10 ↓2`）：用量口径 ⇒ 12/128000 = 0.0%。
                        // 基数用 ref `utils/estimate.ts:18-20` 的 `calculateContextTokens`：
                        // `totalTokens || input+output+cacheRead+cacheWrite`（**含 cache**）。
                        Some(
                            pi_ai::calculate_context_tokens(&assistant.usage) as f64
                                / self.context_window as f64
                                * 100.0,
                        )
                    } else {
                        // 无 usage（T05 的 markdown 夹具）：系统提示 + 本条消息的估算口径
                        // （`footer.ts:112-113` 的 `contextUsage.percent`；T05 golden 显示 1.1%）。
                        let text_tokens = pi_core::agent::compaction::estimate_tokens(
                            &pi_ai::message::Message::Assistant(assistant.clone()),
                        );
                        let system_tokens =
                            pi_core::agent::compaction::estimate_tokens(&self.system_message);
                        // 与 ref `compaction.ts:218-246 estimateContextTokens` 同口径：
                        // 没有 usage 时百分比按**整个投影**（系统提示 + 会话里全部消息）估算，
                        // 而不是只算最后一条 assistant（旧实现少算 user/toolResult ⇒ 1.1% 而 TS 1.2%）。
                        // 投影在提交本轮输入时算好并放进 sink（`set_context_tokens`）。
                        let projected = self
                            .context_tokens
                            .load(std::sync::atomic::Ordering::Relaxed);
                        let tokens = if projected > 0 {
                            projected
                        } else {
                            system_tokens + text_tokens
                        };
                        if projected == 0 {
                            let _ = text_tokens;
                        }
                        Some(tokens as f64 / self.context_window as f64 * 100.0)
                    };
                    // 有错误/截断提示时优先把它送进会话区（否则用户看不到任何反馈）。
                    if std::env::var_os("PI_TUI_DUMP_TRANSCRIPT").is_some() {
                        if let Ok(mut file) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open("/tmp/p78c-events.log")
                        {
                            use std::io::Write as _;
                            let _ = writeln!(
                                file,
                                "MessageEnd stop={:?} notice={:?}",
                                assistant.stop_reason,
                                notice_delta.is_some()
                            );
                        }
                    }
                    let is_retry_round = self
                        .retry_attempt
                        .load(std::sync::atomic::Ordering::Relaxed)
                        > 0;
                    let notice_delta = if is_retry_round { None } else { notice_delta };
                    if let Some(delta) = notice_delta {
                        // 提示是**独立**块（ref `assistant-message.ts:182-199`：`Spacer(1)` 之后
                        // 的 `Text`）。早先走 `AssistantDelta` 会把它拼在正文同一行
                        // （"Hello Error: …"），与 ref 的"空行 + 提示"不一致。
                        let text = delta.trim_start_matches('\n').to_string();
                        if self.tx.send(TuiEvent::AssistantNotice(text)).is_err() {
                            return;
                        }
                    }
                    Some(TuiEvent::Usage {
                        input: assistant.usage.input,
                        output: assistant.usage.output,
                        percent,
                        window,
                        auto: true,
                        cost: None,
                    })
                }
                other => {
                    let _ = other;
                    None
                }
            },
            AgentEvent::MessageUpdate {
                assistant_event, ..
            } => match assistant_event {
                Some(pi_ai::AssistantEvent::TextDelta { delta, .. }) => {
                    Some(TuiEvent::AssistantDelta(delta))
                }
                // P7-6ao/T：thinking 增量单独进会话区（ref 默认**显示** thinking，见 `settings-manager.ts:978`）。
                Some(pi_ai::AssistantEvent::ThinkingDelta { delta, .. }) => {
                    Some(TuiEvent::ThinkingDelta(delta))
                }
                Some(pi_ai::AssistantEvent::Start)
                | Some(pi_ai::AssistantEvent::TextStart { .. })
                | Some(pi_ai::AssistantEvent::TextEnd { .. })
                | Some(pi_ai::AssistantEvent::ThinkingStart { .. })
                | Some(pi_ai::AssistantEvent::ThinkingEnd { .. })
                | None => None,
                Some(other) => {
                    let _ = other;
                    None
                }
            },
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => Some(TuiEvent::ToolCall {
                id: tool_call_id,
                name: tool_name,
                summary: tool_summary(&args),
            }),
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name: _,
                result,
                is_error,
            } => {
                // 结果正文：把所有 text 块按换行拼接（ref `render-utils.ts` 的
                // `getTextOutput` 同口径），图片块在 TUI 里略过。
                let mut parts: Vec<String> = Vec::new();
                // 形状是 `{"content":[{type:"text",text:…}]}`（`run.rs::tool_result_json`
                // 的形态；不是裸数组），`details` 同层。
                let blocks = result
                    .get("content")
                    .and_then(|content| content.as_array())
                    .or_else(|| result.as_array());
                if let Some(blocks) = blocks {
                    for block in blocks {
                        if block.get("type").and_then(|kind| kind.as_str()) == Some("text") {
                            if let Some(text) = block.get("text").and_then(|text| text.as_str()) {
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
                Some(TuiEvent::ToolResult {
                    id: tool_call_id,
                    output: parts.join("\n"),
                    is_error,
                })
            }
            AgentEvent::AgentStart => Some(TuiEvent::Working(true)),
            AgentEvent::AgentSettled => Some(TuiEvent::Working(false)),
            AgentEvent::AgentEnd { .. }
            | AgentEvent::EntryAppended { .. }
            | AgentEvent::CompactionStart { .. }
            | AgentEvent::CompactionEnd { .. }
            | AgentEvent::SummarizationRetryScheduled { .. }
            | AgentEvent::SummarizationRetryAttemptStart { .. }
            | AgentEvent::SummarizationRetryFinished
            | AgentEvent::TurnStart
            | AgentEvent::TurnEnd { .. }
            | AgentEvent::QueueUpdate
            | AgentEvent::MessageStart { .. }
            | AgentEvent::ToolExecutionUpdate { .. }
            | AgentEvent::AutoRetryStart { .. }
            | AgentEvent::AutoRetryEnd { .. } => None,
        };
        if let Some(tui_event) = mapped {
            // 接收端关闭（TUI 已退出）不是错误：丢弃即可。
            let _ = self.tx.send(tui_event);
        }
    }
}

#[cfg(test)]
mod r3_wiring_tests {
    use super::{startup_bytes, take_exit_once};

    /// 启动字节：r3 **不含** `?1049h`；R1/R2 含。
    #[test]
    fn startup_bytes_never_enter_alt_screen_for_r3() {
        let r3 = std::str::from_utf8(startup_bytes()).expect("启动字节不是 UTF-8");
        assert!(
            !r3.contains("\x1b[?1049h"),
            "r3 启动不应进入备用屏幕：{r3:?}"
        );
        // Unix 请求括号粘贴；Windows **不请求**（ConPTY 会把标记拆坏并漏出 `1~`）。
        #[cfg(not(windows))]
        assert!(
            r3.contains("\x1b[?2004h"),
            "Unix 启动应开启括号粘贴：{r3:?}"
        );
        #[cfg(windows)]
        assert!(
            !r3.contains("\x1b[?2004h"),
            "Windows 启动不应请求括号粘贴（ConPTY 会拆坏标记）：{r3:?}"
        );
        // P7-6ae/S2b：kitty 查询三元组已接线（应答由协商层消费），顺序 2004h → kitty → 25l。
        // Windows 不请求括号粘贴（见上面的平台断言），因此只在该平台校验 hideCursor 存在。
        #[cfg(not(windows))]
        {
            let paste_at = r3.find("\x1b[?2004h").expect("缺 2004h");
            let hide_at = r3.find("\x1b[?25l").expect("缺 hideCursor");
            assert!(paste_at < hide_at, "启动顺序应为 2004h → 25l：{r3:?}");
        }
        #[cfg(windows)]
        assert!(r3.contains("\x1b[?25l"), "Windows 启动应隐藏光标：{r3:?}");
        // Windows 不发 kitty 查询（没有协商层消费应答），只在 Unix 断言该三元组。
        #[cfg(not(windows))]
        {
            let kitty_at = r3
                .find("\x1b[>7u\x1b[?u\x1b[c")
                .expect("缺 kitty 查询三元组");
            let paste_at = r3.find("\x1b[?2004h").expect("缺 2004h");
            let hide_at = r3.find("\x1b[?25l").expect("缺 hideCursor");
            assert!(
                paste_at < kitty_at && kitty_at < hide_at,
                "启动顺序应为 2004h → kitty → 25l：{r3:?}"
            );
        }
    }

    /// 退出幂等：连续调用只放行第一次（正常退出 + 信号 + panic hook 共用）。
    #[test]
    fn exit_gate_is_idempotent() {
        let mut restored = false;
        assert!(take_exit_once(&mut restored), "第一次应放行");
        assert!(!take_exit_once(&mut restored), "第二次应被拦下");
        assert!(!take_exit_once(&mut restored), "第三次应被拦下");
    }
}

/// P7-6p(续4)/W1：**诊断转储**（编译期 feature `diag-frame-dump`，默认不含）。
///
/// 只在 r3 路径、只写文件（`$PI_RS_DUMP_DIR/frames.jsonl`），**不写 stdout/stderr**，不改变渲染字节。
#[cfg(feature = "diag-frame-dump")]
mod diag_dump {
    use std::io::Write;

    /// 记录一帧：`seq`、宽高、行数、行哈希、`DiffState` 摘要、写出字节数；`lines.len() > height` 时附完整 lines。
    pub fn dump_frame(
        seq: usize,
        width: usize,
        height: usize,
        lines: &[String],
        state: &pi_tui::diff::DiffState,
        written: usize,
    ) {
        let Some(dir) = std::env::var_os("PI_RS_DUMP_DIR") else {
            return;
        };
        let path = std::path::Path::new(&dir).join("frames.jsonl");
        let hash = {
            // FNV-1a（稳定、无需依赖）
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for line in lines {
                for byte in line.as_bytes() {
                    h ^= u64::from(*byte);
                    h = h.wrapping_mul(0x1000_0000_01b3);
                }
                h ^= 0xff;
                h = h.wrapping_mul(0x1000_0000_01b3);
            }
            h
        };
        let mut body = format!(
            "{{\"seq\":{seq},\"width\":{width},\"height\":{height},\"lines\":{},\"hash\":{hash},\"viewport_top\":{},\"hardware_cursor_row\":{},\"prev_lines\":{},\"written\":{written}",
            lines.len(),
            state.previous_viewport_top,
            state.hardware_cursor_row,
            state.previous_lines.len()
        );
        if lines.len() > height {
            body.push_str(",\"full_lines\":[");
            body.push_str(
                &lines
                    .iter()
                    .map(|l| format!("{l:?}"))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            body.push(']');
        }
        body.push('}');
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(file, "{body}");
        }
    }
}

#[cfg(test)]
mod p76ab_tests {
    use super::r3_exit_bytes;

    /// S2：r3 退出顺序 `2004l` → `<u` →（可选）`>4;0m` → `25h`。
    #[test]
    fn r3_exit_order_matches_ref() {
        let plain = r3_exit_bytes(true, false);
        let paste = plain.find("\x1b[?2004l").expect("缺 2004l");
        let pop = plain.find("\x1b[<u").expect("缺 kitty pop");
        let cursor = plain.find("\x1b[?25h").expect("缺 25h");
        assert!(
            paste < pop && pop < cursor,
            "顺序应为 2004l → <u → 25h：{plain:?}"
        );
        assert!(
            !plain.contains("\x1b[>4;0m"),
            "未开 modifyOtherKeys 时不应写 >4;0m"
        );
        let with_mok = r3_exit_bytes(true, true);
        assert!(
            with_mok.contains("\x1b[>4;0m"),
            "开过 modifyOtherKeys 时应写 >4;0m"
        );
        let not_pushed = r3_exit_bytes(false, false);
        assert!(!not_pushed.contains("\x1b[<u"), "未 push 过时不应写 <u");
    }

    /// S2b：r3 启动含 kitty 三元组（应答由协商层消费，实测无泄漏）。
    /// Windows 不发该查询（没有协商层消费应答，见 `startup_bytes`），故只在 Unix 断言。
    #[test]
    #[cfg(not(windows))]
    fn r3_startup_contains_kitty_triplet() {
        use super::startup_bytes;

        let bytes = std::str::from_utf8(startup_bytes()).expect("不是 UTF-8");
        assert!(
            bytes.contains("\x1b[>7u\x1b[?u\x1b[c"),
            "缺 kitty 查询三元组：{bytes:?}"
        );
        assert!(
            !bytes.contains("\x1b[?1049h"),
            "r3 不应进入备用屏幕：{bytes:?}"
        );
    }
}

/// P7-6ae/S3：**r3 的输入线程**——原始字节 → `input.rs` → 键名映射 → `TuiEvent`。
///
/// 与 crossterm 路径的差别：解析完全由 `pi_tui::input`（StdinBuffer 端口）负责，
/// 到期（孤立 ESC 10 ms / 序列 100 ms）由本线程按 `wake` 显式驱动 `flush()`。
/// `input.rs` / `term_negotiate` / `osc11` 的串联留到 S4。
/// Windows 按键 → `TuiEvent`。
///
/// 两条真机实测约束（2026-10-29，conhost + `WriteConsoleInputW` 注入真按键）：
/// - **只接受 `KeyEventKind::Press`**：crossterm 0.28 对 Windows 的 key-up 记录也产出
///   `Release`（`event/sys/windows/parse.rs:286-292`）；不过滤的话每个字符会被插入两次
///   （实测：只发 down 的 `abc` → `abc`，发 down+up 的 `xyz` → `xxyyzz`，`Ctrl+P` 也一次切两个）。
/// - **`Shift+Tab` 到达时就是 `KeyCode::BackTab`**（同一处 `VK_TAB if SHIFT`），不是
///   `Tab` + SHIFT；只写后者会落到 `_other => None` 被丢掉（真机 `Shift+Tab` 全无反应）。
#[cfg(any(windows, test))]
fn map_windows_key(key: crossterm::event::KeyEvent) -> Option<TuiEvent> {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};

    if key.kind != KeyEventKind::Press {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char(ch) if ctrl => match ch.to_ascii_lowercase() {
            'c' => Some(TuiEvent::Clear),
            'd' => Some(TuiEvent::CtrlD),
            'p' => Some(TuiEvent::CtrlP),
            'n' => Some(TuiEvent::CtrlN),
            's' => Some(TuiEvent::CtrlS),
            'o' => Some(TuiEvent::CtrlO),
            'l' => Some(TuiEvent::CtrlL),
            't' => Some(TuiEvent::CtrlT),
            'z' => Some(TuiEvent::CtrlZ),
            'a' => Some(TuiEvent::LineStart),
            'e' => Some(TuiEvent::LineEnd),
            'k' => Some(TuiEvent::DeleteToLineEnd),
            'u' => Some(TuiEvent::DeleteToLineStart),
            'w' => Some(TuiEvent::DeleteWordBackward),
            'j' => Some(TuiEvent::Newline),
            _unmapped => None,
        },
        KeyCode::Char(ch) => Some(TuiEvent::Char(ch)),
        // ref `packages/tui/src/keybindings.ts:143`：`tui.input.newLine = ["shift+enter", "ctrl+j"]`。
        // - **Shift+Enter** 是 ref 明确绑定的换行键，必须插入换行而不是提交（W-Q3 复核发现此前漏了）；
        // - **Ctrl+Enter** 是 ConPTY 对"粘贴里的换行"的编码，也必须按换行处理，
        //   不能当成提交（否则粘贴结束前的裸回车会直接提交）；
        // - `Ctrl+J` 由下面的 `Char('j') + CONTROL ⇒ Newline` 覆盖（同样来自 ref:143）。
        KeyCode::Enter if ctrl || key.modifiers.contains(KeyModifiers::SHIFT) => {
            Some(TuiEvent::Newline)
        }
        KeyCode::Enter => Some(TuiEvent::Enter),
        KeyCode::Backspace => Some(TuiEvent::Backspace),
        KeyCode::Esc => Some(TuiEvent::Escape),
        KeyCode::Left => Some(TuiEvent::CursorLeft),
        KeyCode::Right => Some(TuiEvent::CursorRight),
        KeyCode::Up => Some(TuiEvent::HistoryPrev),
        KeyCode::Down => Some(TuiEvent::HistoryNext),
        KeyCode::PageUp => Some(TuiEvent::PageUp),
        KeyCode::PageDown => Some(TuiEvent::PageDown),
        KeyCode::BackTab => Some(TuiEvent::BackTab),
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => Some(TuiEvent::BackTab),
        KeyCode::Tab => Some(TuiEvent::Tab),
        KeyCode::Home => Some(TuiEvent::LineStart),
        KeyCode::End => Some(TuiEvent::LineEnd),
        KeyCode::Delete => Some(TuiEvent::Delete),
        _other => None,
    }
}

/// 终端查询应答的过滤状态（Windows 专用）。
///
/// Windows 控制台把我们启动时发出的查询应答（DA1 `\x1b[?1;0c`、kitty `\x1b[?…u`、
/// OSC 11 颜色应答）也写进输入缓冲，crossterm 只把它当作一串 `KeyCode::Char` ⇒ 字符会被
/// 插进编辑器（真机实测：启动后编辑框里已有 `[?1;0c`，导致提示词被污染、`Ctrl+D`/`Ctrl+L`
/// 因 `input` 非空而失效）。Unix 的 `input.rs` 自带协商层，这里补一个最小等价实现。
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum RespState {
    #[default]
    Idle,
    /// 收到 ESC，尚未确定是按键还是应答。
    AfterEsc,
    /// `ESC [` … 等 CSI 终止字节。
    Csi,
    /// `ESC ]` … 等 BEL / ST。
    Osc,
    /// OSC 内的 ESC，等 `\\`。
    OscEsc,
    /// `ESC O` 后的单个字符。
    Ss3,
}

/// [`RespFilter::feed`] 的结果。
#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum FilterOut {
    /// 丢弃（应答的一部分，或 ESC 还在等后续字符）。
    Nothing,
    /// 普通字符：原样发出。
    Char(char),
    /// 是用户的 Esc 键（不是应答）。
    Escape,
    /// 用户的 Esc 键 + 跟随字符。
    EscapeThenChar(char),
    /// 不是终端应答的 CSI/SS3 序列：**整段原样回放**（不能只吐后半截，
    /// 否则 `ESC[201~` 会漏成 `1~`——P8e-17 真机症状）。
    Sequence(String),
}

/// 应答过滤状态机（纯逻辑，可单测）。
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Default)]
struct RespFilter {
    state: RespState,
    len: usize,
    /// Csi/Ss3 已收到的字节（含 `ESC[` 前缀），用于判定"应答"还是"回放"。
    seq: String,
}

#[cfg(any(windows, test))]
impl RespFilter {
    /// 单条序列的最大长度（防御无终止符的脏数据）。
    const MAX_LEN: usize = 64;

    /// 收到 Esc 键：先挂起，等下一个字符决定去向。
    ///
    /// OSC 序列里的 ESC 多半是 ST（`ESC \`）的前半段，不能当成用户的 Esc 键重置状态，
    /// 否则 `]11;rgb:…\` 这种应答会漏出 `Escape` + `Char('\')`。
    fn on_escape(&mut self) {
        self.state = match self.state {
            RespState::Osc | RespState::OscEsc => RespState::OscEsc,
            RespState::Idle | RespState::AfterEsc | RespState::Csi | RespState::Ss3 => {
                self.len = 0;
                self.seq.clear();
                RespState::AfterEsc
            }
        };
    }

    /// 超时（没有后续输入）时，挂起的 Esc 就是用户按的 Esc。
    ///
    /// **只能**在 `AfterEsc` 时放行：此前无条件 `state = Idle`，于是一个慢速的
    /// `ESC[201~`（ConPTY 逐字节、两字节间隔 > 10 ms）会在 tick 里被重置成 Idle，
    /// 已经吞掉的 `ESC[20` 不会补回，剩下的 `1~` 就当普通字符进编辑器
    /// —— 这正是真机看不到字符却多出 `1~` 的原因。
    fn take_pending_escape(&mut self) -> bool {
        if self.state == RespState::AfterEsc {
            self.state = RespState::Idle;
            self.len = 0;
            true
        } else {
            false
        }
    }

    /// 这条 CSI/SS3 序列是不是"终端对我们查询的应答"（应吞掉）。
    ///
    /// 只认已知形态；其余一律回放，避免把 `ESC[201~` 这类标记的尾巴吃掉。
    fn is_terminal_reply(sequence: &str) -> bool {
        let body = sequence
            .strip_prefix("\u{1b}[")
            .or_else(|| sequence.strip_prefix("\u{1b}O"));
        let Some(body) = body else {
            return false;
        };
        // DA1 应答：`?1;0c` 之类；kitty 键盘：`?<flags>u` / `><flags>u`；
        // 光标位置报告：`<row>;<col>R`；设备属性：`?…c`。
        body.starts_with('?') || body.starts_with('>') || body.ends_with('R')
    }

    /// 喂一个字符，返回该发出的内容。
    fn feed(&mut self, ch: char) -> FilterOut {
        match self.state {
            RespState::Idle => FilterOut::Char(ch),
            RespState::AfterEsc => match ch {
                '\x1b' => {
                    self.state = RespState::AfterEsc;
                    self.len = 0;
                    FilterOut::Escape
                }
                '[' => {
                    self.state = RespState::Csi;
                    self.len = 0;
                    self.seq.clear();
                    self.seq.push_str("\u{1b}[");
                    FilterOut::Nothing
                }
                ']' => {
                    self.state = RespState::Osc;
                    self.len = 0;
                    FilterOut::Nothing
                }
                'O' => {
                    self.state = RespState::Ss3;
                    self.seq.clear();
                    self.seq.push_str("\u{1b}O");
                    FilterOut::Nothing
                }
                other => {
                    self.state = RespState::Idle;
                    FilterOut::EscapeThenChar(other)
                }
            },
            RespState::Csi => {
                self.len = self.len.saturating_add(1);
                self.seq.push(ch);
                let terminated = ('\x40'..='\x7e').contains(&ch) || self.len > Self::MAX_LEN;
                if !terminated {
                    return FilterOut::Nothing;
                }
                self.state = RespState::Idle;
                let sequence = std::mem::take(&mut self.seq);
                if Self::is_terminal_reply(&sequence) {
                    // 确实是终端应答（DA1 / kitty / 光标报告等）：整段吞掉
                    FilterOut::Nothing
                } else {
                    // 不是应答：**整段原样回放**，绝不只吐尾巴
                    FilterOut::Sequence(sequence)
                }
            }
            RespState::Ss3 => {
                // SS3（`ESC O P` = F1、`ESC O A` = 上行…）永远是**按键**，不是正文；
                // 这里只负责不让它们变成文本。按键事件由 `map_windows_key` 处理。
                self.state = RespState::Idle;
                self.seq.clear();
                FilterOut::Nothing
            }
            RespState::Osc => {
                self.len = self.len.saturating_add(1);
                if ch == '\x07' || self.len > Self::MAX_LEN {
                    self.state = RespState::Idle;
                } else if ch == '\x1b' {
                    self.state = RespState::OscEsc;
                }
                FilterOut::Nothing
            }
            RespState::OscEsc => {
                self.state = if ch == '\\' {
                    RespState::Idle
                } else {
                    RespState::Osc
                };
                FilterOut::Nothing
            }
        }
    }
}

/// Windows 粘贴的“突发”合并（真机缺陷：多行粘贴会直接提交）。
///
/// 经典 conhost 的 Shift+Insert / 右键粘贴**不是** bracketed paste：它把整段文本拆成一串
/// `KEY_EVENT` 逐个送进来，行间是普通回车 ⇒ 第一行末尾的回车就被当成“提交”，
/// 整段粘贴变成多次对话（真机实测：粘贴 `L1⏎L2⏎L3⏎` 直接把 `L1` 发出）。
///
/// 判定办法是**回车后回看**：收到回车先挂起（不提交），`LOOKAHEAD_MS` 内还有字符进来
/// ⇒ 这是粘贴的换行，继续攒；超时没有后续输入 ⇒ 才把挂起的回车当成提交。
/// 只靠“字符间隔窗口”不够：实测 12 行粘贴会被 conhost 分块送达，块间间隔会超过窗口。
#[cfg(any(windows, test))]
#[derive(Debug, Default, Clone)]
struct PasteBurst {
    chars: Vec<char>,
    /// 本段突发第一个字符的时刻（用于判定“这是粘贴而不是打字”）。
    first_ms: u64,
    /// 挂起的回车（回看中，尚未决定是“粘贴换行”还是“提交”）。
    enter_pending: bool,
    last_ms: u64,
    /// 刚收到一个**普通 Enter（CR）**，还在等它是不是 CRLF 的前半。
    pending_cr: bool,
    /// 挂起的回车是 `LF`（`Ctrl+Enter`／用户 ctrl+j）——提交判定时要发 `Newline` 而不是 `Enter`。
    pending_is_lf: bool,
}

#[cfg(any(windows, test))]
impl PasteBurst {
    /// 同一段粘贴里字符之间的最大间隔（也是单字的合并窗口：
    /// 真机实测 conhost 首次送达可能比后续块晚十几毫秒，取 12 ms 会把首字符切出去；
    /// 人工打字 ≳ 60 ms/字，30 ms 不影响判定，代价是击键显示最多晚 30 ms）。
    const WINDOW_MS: u64 = 30;
    /// 回车之后的回看窗口：这段时间内还有字符 ⇒ 该回车是粘贴里的换行。
    const LOOKAHEAD_MS: u64 = 60;
    /// 缓冲上限（防御无换行的超大粘贴）。
    const MAX_CHARS: usize = 4096;
    /// ref `editor.ts:1296-1312` 的折叠阈值之一（>1000 字符）。
    const PASTE_FOLD_THRESHOLD: usize = 1000;
    /// 超过这个长度就认定为粘贴（人类打字达不到），改用更宽的空闲窗口。
    const LARGE_BURST: usize = 64;
    /// 大突发的空闲窗口（毫秒）。实测 1200 字符粘贴里偶发 >200 ms 的调度停顿，
    /// 会把一段切成两段（两段都不到折叠阈值 ⇒ 不折叠），故取 600 ms。
    const LARGE_BURST_IDLE_MS: u64 = 600;
    /// 判定“像粘贴”的平均按键间隔上限（毫秒）：人类打字不可能 5 ms/字。
    const PASTE_AVG_GAP_MS: u64 = 5;

    /// 收入一个字符；返回“上一段需要先发出的事件”（间隔超过窗口 ⇒ 两段不是同一次粘贴）。
    fn on_char(&mut self, ch: char, now_ms: u64) -> Option<Vec<TuiEvent>> {
        if self.pending_cr {
            // CR 后面不是 LF ⇒ 那个 CR 是真正的换行
            self.pending_cr = false;
            self.push_newline(true, now_ms);
        }
        if self.enter_pending {
            // 回车后紧跟字符 ⇒ 该回车是粘贴里的换行；但缓冲区为空时它是杂散换行
            // （启动残留 / 粘贴首尾），吸收掉不产生空行。
            if !self.chars.is_empty() {
                self.chars.push('\n');
            }
            self.enter_pending = false;
        }
        let mut flushed = if !self.chars.is_empty()
            && now_ms.saturating_sub(self.last_ms) > self.idle_window()
            && !self.chars.contains(&'\n')
        {
            Some(self.take())
        } else {
            None
        };
        if self.chars.is_empty() {
            self.first_ms = now_ms;
        }
        self.chars.push(ch);
        self.last_ms = now_ms;
        if self.chars.len() >= Self::MAX_CHARS {
            let tail = self.take();
            return Some(match flushed.as_mut() {
                Some(events) => {
                    events.extend(tail);
                    std::mem::take(events)
                }
                None => tail,
            });
        }
        flushed
    }

    /// 收入一个回车。
    ///
    /// 紧跟刚到的字符（≤ `WINDOW_MS`）⇒ 这是粘贴里的换行，直接并进缓冲；
    /// 否则挂起，交给 [`Self::tick`] 的回看窗口裁决（粘贴分块送达时空隙可能大于窗口）。
    /// 收入一个回车。`is_line_feed` 为真表示这是 `Ctrl+Enter`（ConPTY 对 `LF` 的编码）。
    ///
    /// P8e-17b：剪贴板的换行是 **CRLF**，终端把它拆成两个事件
    /// （真机日志 3418ms 普通 Enter + 3420ms Ctrl+Enter，间隔 2 ms）：
    /// - `CR` + `LF`（1-2 ms 内成对）⇒ **一个**换行；
    /// - `CR LF CR LF` ⇒ **两个**换行（空行不能被吞，这是上一次修过分的关键）。
    ///
    /// 因此只按"CR 是否紧跟 LF"配对，**不看时间窗**：`LF` 自带换行语义，
    /// `CR` 只是待定的前半段；若后面不是 LF 就补一个换行。
    fn on_enter(&mut self, is_line_feed: bool, now_ms: u64) {
        if self.pending_cr {
            self.pending_cr = false;
            // 前一个 CR 到此结算：CR 单独出现也是换行（老 Mac 风格或真按回车）
            self.push_newline(is_line_feed, now_ms);
            if !is_line_feed {
                // 连续两个 CR：当前这个继续待定
                self.pending_cr = true;
            }
            return;
        }
        if is_line_feed {
            self.push_newline(true, now_ms);
            return;
        }
        self.pending_cr = true;
    }

    /// 把一个已经确定是"换行"的回车并进缓冲区（或挂起等提交判定）。
    fn push_newline(&mut self, is_lf: bool, now_ms: u64) {
        let immediate =
            !self.chars.is_empty() && now_ms.saturating_sub(self.last_ms) <= Self::WINDOW_MS;
        self.last_ms = now_ms;
        if immediate {
            self.chars.push('\n');
            self.enter_pending = false;
            return;
        }
        if self.enter_pending && !self.chars.is_empty() {
            self.chars.push('\n');
        }
        // 缓冲区为空时挂起的换行是"杂散回车"（启动残留/粘贴首尾），直接吸收
        self.enter_pending = true;
        self.pending_is_lf = is_lf;
    }

    /// 定时驱动（poll 超时）：回看窗口到期 ⇒ 决定挂起的回车是“粘贴换行”还是“提交”。
    /// 当前这段突发允许的空闲间隔（毫秒）。
    ///
    /// 大突发（≥ `LARGE_BURST` 字符）说明是粘贴而非打字（人类 30 ms 打不出 64 个字），
    /// 此时把窗口放宽到 200 ms：实测 1200 字符粘贴偶发因调度停顿被切成两段，
    /// 切开后每段都不到折叠阈值，折叠就不会触发（W-Q3 真机 5 次里出现 1 次）。
    fn idle_window(&self) -> u64 {
        if self.chars.len() >= Self::LARGE_BURST && self.looks_like_paste() {
            Self::LARGE_BURST_IDLE_MS
        } else {
            Self::WINDOW_MS
        }
    }

    /// 平均按键间隔 <= `PASTE_AVG_GAP_MS` 且长度够 ⇒ 是粘贴（人类打字达不到这个速率）。
    fn looks_like_paste(&self) -> bool {
        if self.chars.len() < 2 {
            return false;
        }
        let span = self.last_ms.saturating_sub(self.first_ms);
        span / (self.chars.len() as u64 - 1) <= Self::PASTE_AVG_GAP_MS
    }

    fn tick(&mut self, now_ms: u64) -> Option<Vec<TuiEvent>> {
        if self.pending_cr && now_ms.saturating_sub(self.last_ms) >= Self::LOOKAHEAD_MS {
            // CR 已经等满一个回看窗：确定它是"真回车"（不是 CRLF 前半），
            // 直接按纯回车裁决（空缓冲 ⇒ 提交），不必再等一轮。
            self.pending_cr = false;
            self.enter_pending = true;
            self.pending_is_lf = false;
        }
        let idle = now_ms.saturating_sub(self.last_ms);
        if self.enter_pending && idle >= Self::LOOKAHEAD_MS {
            self.enter_pending = false;
            if self.chars.is_empty() {
                // 纯 `LF`（用户按 ctrl+j / ConPTY 的 Ctrl+Enter）⇒ 插入换行；
                // 纯 `CR` ⇒ 提交。
                return Some(vec![if self.pending_is_lf {
                    TuiEvent::Newline
                } else {
                    TuiEvent::Enter
                }]);
            }
            if self.chars.contains(&'\n') {
                // 多行内容后面的回车 ⇒ 粘贴末尾的换行，并入粘贴、不提交。
                self.chars.push('\n');
                return Some(self.take());
            }
            // 单行内容 + 回车 ⇒ 提交。
            let mut events = self.take();
            events.push(TuiEvent::Enter);
            return Some(events);
        }
        if self.enter_pending {
            return None;
        }
        if !self.chars.is_empty() && idle > self.idle_window() {
            return Some(self.take());
        }
        None
    }

    /// 收到非字符事件（Ctrl+*、方向键等）：立刻按 [`Self::tick`] 的同一套规则结清缓冲。
    #[cfg_attr(not(windows), allow(dead_code))]
    fn take_all(&mut self) -> Vec<TuiEvent> {
        if self.pending_cr {
            self.pending_cr = false;
            self.push_newline(true, self.last_ms);
        }
        if !self.enter_pending {
            return self.take();
        }
        self.enter_pending = false;
        if self.chars.is_empty() {
            return vec![if self.pending_is_lf {
                TuiEvent::Newline
            } else {
                TuiEvent::Enter
            }];
        }
        if self.chars.contains(&'\n') {
            self.chars.push('\n');
            return self.take();
        }
        let mut events = self.take();
        events.push(TuiEvent::Enter);
        events
    }

    /// 取出缓冲内容（不清 `enter_pending`）。
    /// 是否有待决内容（正在攒的字符或挂起的回车）。
    fn is_pending(&self) -> bool {
        !self.chars.is_empty() || self.enter_pending || self.pending_cr
    }

    fn take(&mut self) -> Vec<TuiEvent> {
        let chars = std::mem::take(&mut self.chars);
        self.first_ms = 0;
        // 多行，或长度超过 ref 的折叠阈值（`editor.ts:1296-1312`：>1000 字符）⇒ 当成一次粘贴。
        // 后者是 W-Q3 真机发现的缺口：单行 1200 字符粘贴没有换行，旧判定会逐字符插入，
        // 永远触发不了“折叠”（人类不可能在 30 ms 的突发窗口里打出 1000 个字符）。
        if chars.contains(&'\n') || chars.len() > Self::PASTE_FOLD_THRESHOLD {
            vec![TuiEvent::Paste(chars.into_iter().collect())]
        } else {
            chars.into_iter().map(TuiEvent::Char).collect()
        }
    }
}

/// Windows 输入线程：crossterm `event::read` → `TuiEvent`。
///
/// Windows 控制台的 VT 输入模式对 `Ctrl+<字母>` 的字节转换不可靠（真机实测
/// `Ctrl+D` 收不到 `0x04`、`Ctrl+P` 收不到 `0x10`），而 crossterm 直接给出
/// `KeyCode::Char` + `CONTROL`。Unix 仍走字节路径（与 golden 逐字节对齐）。
/// 这里还要额外做两件事：只收 `Press`（见 [`map_windows_key`]）、抹掉终端查询应答
/// （见 [`RespFilter`]）。
/// Windows 输入线程的**纯逻辑**部分（可注入时钟，Linux 上也能单测）。
///
/// 组合三件事：`RespFilter`（抹掉 DA1 应答）、`PasteBurst`（conhost 逐键粘贴合批）、
/// `BracketPaste`（ConPTY 把 `ESC[200~`/`ESC[201~` 拆成按键时的重组）。
#[cfg(any(windows, test))]
#[derive(Default)]
pub struct WindowsInput {
    filter: RespFilter,
    burst: PasteBurst,
    bracket: crate::bracket_paste::BracketPaste,
}

#[cfg(any(windows, test))]
impl WindowsInput {
    /// 喂一个 crossterm 按键事件。
    pub fn on_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        now_ms: u64,
    ) -> Vec<pi_tui::TuiEvent> {
        use crate::bracket_paste::{BracketInput, BracketOut};
        if key.kind == crossterm::event::KeyEventKind::Press {
            let input = match key.code {
                crossterm::event::KeyCode::Esc => BracketInput::Esc,
                crossterm::event::KeyCode::Char(ch) => BracketInput::Char(ch),
                // ConPTY 在括号粘贴内部把换行送成 Ctrl+Enter：收集时必须当成 '\n'，
                // 否则会被判成"其它按键"从而中断整段收集。
                crossterm::event::KeyCode::Enter
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    BracketInput::Char('\n')
                }
                // ConPTY 把粘贴文本里的 tab 还原成 VK_TAB 键事件：收集期它是**正文的一部分**
                // （ref `editor.ts:1146` 把 tab 展开成 4 空格），不能当成“其它按键”中断收集。
                // 空闲时 `feed` 会原样 `Pass` 回来，仍按 `TuiEvent::Tab`（补全）处理。
                crossterm::event::KeyCode::Tab => BracketInput::Char('\t'),
                _other => BracketInput::Other,
            };
            match self.bracket.feed(input, now_ms) {
                BracketOut::Consumed => return Vec::new(),
                BracketOut::Paste(text) => {
                    let mut emitted = self.burst.take_all();
                    emitted.push(pi_tui::TuiEvent::Paste(text));
                    return emitted;
                }
                // 暂扣的 Esc 被判定"不是标记"：先把它放出去，再处理当前字符
                BracketOut::EscReleased => return vec![pi_tui::TuiEvent::Escape],
                // 标记残片按正文交回（不静默丢字）。这里是"前缀 + 一个非字符键"的交错，
                // 概率极低；正文优先插入，该按键丢弃（比把 `[20` 漏进正文安全）。
                BracketOut::TextThen(text, _input) => {
                    let mut emitted = self.burst.take_all();
                    emitted.push(pi_tui::TuiEvent::Paste(text));
                    return emitted;
                }
                BracketOut::Pass(_) => {}
            }
        }
        let Some(mapped) = map_windows_key(key) else {
            return Vec::new();
        };
        let mut emitted: Vec<pi_tui::TuiEvent> = Vec::new();
        match mapped {
            pi_tui::TuiEvent::Escape => {
                emitted.extend(self.burst.take_all());
                self.filter.on_escape();
            }
            pi_tui::TuiEvent::Char(ch) => match self.filter.feed(ch) {
                FilterOut::Nothing => {}
                FilterOut::Char(ch) => {
                    if let Some(previous) = self.burst.on_char(ch, now_ms) {
                        emitted.extend(previous);
                    }
                }
                FilterOut::Escape => {
                    emitted.extend(self.burst.take_all());
                    emitted.push(pi_tui::TuiEvent::Escape);
                }
                FilterOut::EscapeThenChar(ch) => {
                    emitted.extend(self.burst.take_all());
                    emitted.push(pi_tui::TuiEvent::Escape);
                    if let Some(previous) = self.burst.on_char(ch, now_ms) {
                        emitted.extend(previous);
                    }
                }
                FilterOut::Sequence(sequence) => {
                    // 非应答的 CSI/SS3（例如漏过括号重组的 `ESC[201~`）：整段
                    // 按普通输入回放，避免只吐尾巴（真机 `1~`）。
                    for ch in sequence.chars() {
                        if let Some(previous) = self.burst.on_char(ch, now_ms) {
                            emitted.extend(previous);
                        }
                    }
                }
            },
            // ConPTY 把粘贴里的换行送成 Ctrl+Enter（见 `map_windows_key`），
            // 映射到 `Newline`：正在攒粘贴 ⇒ 并入这段文本（当 '\n'）；
            // 否则就是用户真按了 ctrl+j ⇒ 直接插入换行（ref keybindings.ts:143）。
            pi_tui::TuiEvent::Newline => {
                // `Ctrl+Enter`（ConPTY 对 LF 的编码）＝真正的换行
                self.burst.on_enter(true, now_ms);
            }
            pi_tui::TuiEvent::Enter => {
                // 普通 Enter：先当 CR 待定，紧跟 LF 时配对成一个换行
                self.burst.on_enter(false, now_ms);
            }
            // 无括号标记的键流粘贴里，tab 是正文的一部分：攒批中当字符收下，
            // 空闲时才是编辑器里的 Tab（补全）。
            pi_tui::TuiEvent::Tab if self.burst.is_pending() => {
                if let Some(previous) = self.burst.on_char('\t', now_ms) {
                    emitted.extend(previous);
                }
            }
            other => {
                emitted.extend(self.burst.take_all());
                if self.filter.take_pending_escape() {
                    emitted.push(pi_tui::TuiEvent::Escape);
                }
                emitted.push(other);
            }
        }
        emitted
    }

    /// 终端直接给出的整段粘贴（crossterm `Event::Paste`）。
    pub fn on_paste(&mut self, text: String) -> Vec<pi_tui::TuiEvent> {
        let mut emitted = self.burst.take_all();
        if self.filter.take_pending_escape() {
            emitted.push(pi_tui::TuiEvent::Escape);
        }
        emitted.push(pi_tui::TuiEvent::Paste(text));
        emitted
    }

    /// 定时 tick（10 ms 粒度）：让挂起的回车与暂扣的 Esc 到期。
    pub fn tick(&mut self, now_ms: u64) -> Vec<pi_tui::TuiEvent> {
        let mut emitted: Vec<pi_tui::TuiEvent> = Vec::new();
        if let Some(events) = self.burst.tick(now_ms) {
            emitted.extend(events);
        }
        if self.filter.take_pending_escape() {
            emitted.push(pi_tui::TuiEvent::Escape);
        }
        if let Some(crate::bracket_paste::BracketOut::EscReleased) = self.bracket.flush(now_ms) {
            emitted.push(pi_tui::TuiEvent::Escape);
        }
        emitted
    }
}

#[cfg(windows)]
fn spawn_crossterm_input_thread(tx: std::sync::mpsc::Sender<TuiEvent>) {
    use crossterm::event::{self, Event};

    std::thread::spawn(move || {
        let mut machine = WindowsInput::default();
        let started = std::time::Instant::now();
        let now_ms = || started.elapsed().as_millis() as u64;
        loop {
            // 10 ms 粒度：既要及时冲掉粘贴突发（12 ms 窗口），也要给挂起的回车到期
            let ready = match event::poll(Duration::from_millis(10)) {
                Ok(ready) => ready,
                Err(_) => return,
            };
            if !ready {
                if machine
                    .tick(now_ms())
                    .into_iter()
                    .any(|event| tx.send(event).is_err())
                {
                    return;
                }
                continue;
            }
            let event = match event::read() {
                Ok(event) => event,
                Err(_) => return,
            };
            // 诊断开关：`PI_TUI_EVENT_DUMP=<path>` 把每个 crossterm 事件（含时间戳）
            // 追加到文件，用于定位真机上的粘贴/输入法异常。
            if let Some(path) = std::env::var_os("PI_TUI_EVENT_DUMP") {
                use std::io::Write as _;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    let _ = writeln!(file, "{:>6}ms  {event:?}", started.elapsed().as_millis());
                }
            }
            let emitted = match event {
                Event::Key(key) => machine.on_key(key, now_ms()),
                Event::Paste(text) => machine.on_paste(text),
                _other => Vec::new(),
            };
            if emitted.into_iter().any(|event| tx.send(event).is_err()) {
                return;
            }
        }
    });
}

#[cfg(not(windows))]
fn spawn_r3_input_thread(tx: std::sync::mpsc::Sender<TuiEvent>) {
    use pi_tui::input::InputState;
    use std::io::Read;

    std::thread::spawn(move || {
        let (byte_tx, byte_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut buf = [0u8; 4096];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if byte_tx.send(buf[..n].to_vec()).is_err() {
                            // 主循环已结束：继续**读并丢弃**最多 200 ms，把终端对我们启动查询的
                            // 应答（kitty `\x1b[?...u`、DA1 `\x1b[...c`）读走。
                            // 否则它们留在 tty 输入队列里，退出后被 shell 当命令执行
                            // （真机表现为 `bash: 1:3ua7: 未找到命令`）。
                            // 真机复验反馈：退出后仍偶发 `bash: 1:3u… 未找到命令`，
                            // 说明终端应答可能在 200 ms 之后才到 ⇒ 放宽到 500 ms（仍有界）。
                            let deadline =
                                std::time::Instant::now() + std::time::Duration::from_millis(500);
                            while std::time::Instant::now() < deadline {
                                match stdin.read(&mut buf) {
                                    Ok(0) | Err(_) => return,
                                    Ok(_discarded) => {}
                                }
                            }
                            return;
                        }
                    }
                }
            }
        });

        let started = std::time::Instant::now();
        let now_ms = || started.elapsed().as_millis() as u64;
        let mut input = InputState::new(false);
        // P7-6ae/S4：协商层（kitty/DA1 应答消费）与 OSC 11（配额应答消费）串在 input.rs 之后。
        let mut negotiate = pi_tui::term_negotiate::NegotiateState::new();
        let mut osc11 = pi_tui::osc11::Osc11State::new();
        let mut wake: Option<u64> = None;

        loop {
            let timeout = match wake {
                Some(at) => {
                    let now = now_ms();
                    std::time::Duration::from_millis(at.saturating_sub(now))
                }
                None => std::time::Duration::from_millis(200),
            };
            let chunk = match byte_rx.recv_timeout(timeout) {
                Ok(chunk) => chunk,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // 计时器到期：把残留缓冲交出来
                    for event in input.flush() {
                        if !forward_event(event, &mut negotiate, &mut osc11, &tx) {
                            return;
                        }
                    }
                    // 协商层的 150 ms 前缀兜底（仅超时可达，保留）
                    for action in negotiate.on_timeout() {
                        if let pi_tui::term_negotiate::Action::Forward(seq) = action {
                            if !forward_sequence(&seq, &mut negotiate, &mut osc11, &tx) {
                                return;
                            }
                        }
                    }
                    wake = None;
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            };
            let (events, next) = input.process(&chunk, now_ms());
            for event in events {
                if !forward_event(event, &mut negotiate, &mut osc11, &tx) {
                    return;
                }
            }
            wake = next;
        }
    });
}

/// 把一个 `InputEvent` 依次过协商层与 OSC 11，再映射成 `TuiEvent`。
///
/// 返回 `false` 表示主循环已退出（通道断开）。
/// 仅 Unix 输入线程使用（Windows 走 crossterm 事件路径），因此在 Windows 目标上不编译。
#[cfg(not(windows))]
fn forward_event(
    event: pi_tui::input::InputEvent,
    negotiate: &mut pi_tui::term_negotiate::NegotiateState,
    osc11: &mut pi_tui::osc11::Osc11State,
    tx: &std::sync::mpsc::Sender<TuiEvent>,
) -> bool {
    use pi_tui::input::InputEvent;
    match event {
        InputEvent::Paste(content) => tx.send(TuiEvent::Paste(content)).is_ok(),
        InputEvent::Data(seq) => forward_sequence(&seq, negotiate, osc11, tx),
    }
}

/// 单个序列：协商层 ⇒（未消费时）OSC 11 ⇒（未消费时）键名映射。
/// 仅 Unix 输入线程使用（同上）。
#[cfg(not(windows))]
fn forward_sequence(
    seq: &str,
    negotiate: &mut pi_tui::term_negotiate::NegotiateState,
    osc11: &mut pi_tui::osc11::Osc11State,
    tx: &std::sync::mpsc::Sender<TuiEvent>,
) -> bool {
    // ref `tui.ts:1073-1076`：**release 事件一律丢弃**（除非组件显式 `wantsKeyRelease`，编辑器不 opt-in）。
    // 真实终端（如 Alacritty）在 kitty 协议下会给每个键发 press+release，不过滤就会**每键触发两次**
    // —— 这正是真机上 "/" 变 "//"、任意字符重复、Ctrl+J 双换行的根因。
    if pi_tui::input::is_release(seq) {
        return true;
    }
    use pi_tui::osc11::Osc11Outcome;
    use pi_tui::term_negotiate::Action;

    let (actions, _wake) = negotiate.on_sequence(seq, 0);
    for action in actions {
        match action {
            Action::Consume => return true, // 协商应答被消费，不产生键事件
            Action::Write(bytes) => {
                let mut out = std::io::stdout();
                let _ = std::io::Write::write_all(&mut out, bytes.as_bytes());
                let _ = std::io::Write::flush(&mut out);
            }
            Action::Forward(seq) => {
                // 协商层放行的序列仍要过 OSC 11
                if matches!(osc11.on_sequence(&seq), Osc11Outcome::Consumed(_)) {
                    return true;
                }
                // DSR 996/997：形状匹配即消费（无配额），`tui.ts:1010`/`:1108-1118`
                if pi_tui::dsr996::on_sequence(&seq).is_some() {
                    return true;
                }
                if let Some(mapped) = map_key_sequence(&seq) {
                    return tx.send(mapped).is_ok();
                }
                return true;
            }
        }
    }
    true
}

/// 键名/序列 → `TuiEvent`（P7-6ab S1.4 的映射表）。
///
/// **未绑定的键按 ref 语义忽略**（`f1..f12`、`insert`、`shift+方向` 等 ref 无默认绑定）。
#[must_use]
pub fn map_key_sequence(seq: &str) -> Option<TuiEvent> {
    // P7-6af/C：`input.rs::parse_key` 已按 `keys.ts:1283-1291` 修正（`\r`/`\t`/`\x7f`/`\x1b[Z` 等），
    // 这里不再需要前置映射。
    // P7-6af/C 后续：ref 里 `\n` **同时**匹配 `ctrl+j`（`tui.input.newLine`）与 `enter`
    // （实测 `matchesKey("\n","ctrl+j") = true`、`matchesKey("\n","enter") = true`），
    // 编辑器按绑定顺序先命中 newLine ⇒ `\n` 是**换行**而不是提交（T09 的 golden 即为两行）。
    if seq == "\n" {
        return Some(TuiEvent::Newline);
    }
    // ref 的两张表要分开看：`keys.ts:373-374` 的 legacy 表（键位匹配用，含
    // `[1~`/`[7~`/`[4~`/`[8~`）与 `parseKey` 的序列表（不含这四个）。tmux 的本机
    // （xterm）Home/End 发的正是 `[1~`/`[4~`（`od -c` 取证），TS 键位层认它们，
    // 所以这里在**键位层**补上；`parse_key` 保持与 oracle 一致（返回 None）。
    match seq {
        "\u{1b}[1~" | "\u{1b}[7~" => return Some(TuiEvent::LineStart),
        "\u{1b}[4~" | "\u{1b}[8~" => return Some(TuiEvent::LineEnd),
        _other => {}
    }
    let key = pi_tui::input::parse_key(seq, false);
    let name = key.as_deref().unwrap_or(seq);
    let mapped = match name {
        "enter" => TuiEvent::Enter,
        "space" => TuiEvent::Char(' '),
        "backspace" => TuiEvent::Backspace,
        "escape" => TuiEvent::Escape,
        "tab" => TuiEvent::Tab,
        // ref `keybindings.ts:100-102`：`app.thinking.cycle` 默认 `shift+tab`。
        "shift+tab" => TuiEvent::BackTab,
        "ctrl+d" => TuiEvent::CtrlD,
        // ref `tree-selector.ts:1080-1085`：`ctrl+o` 是 `/tree` 的过滤循环；主界面是"显示更多"。
        "ctrl+z" => TuiEvent::CtrlZ,
        "ctrl+o" => TuiEvent::CtrlO,
        // ref `core/keybindings.ts:210-231`：`/tree` 的直选过滤键（主界面无绑定）。
        "ctrl+t" => TuiEvent::CtrlT,
        // ref `core/keybindings.ts`：`app.model.cycleForward` 默认 `ctrl+p`、`cycleBackward` 默认 `ctrl+n`。
        "ctrl+p" => TuiEvent::CtrlP,
        "ctrl+s" => TuiEvent::CtrlS,
        "ctrl+n" => TuiEvent::CtrlN,
        "ctrl+r" => TuiEvent::CtrlR,
        "ctrl+x" => TuiEvent::CtrlX,
        "ctrl+left" => TuiEvent::CtrlLeft,
        "ctrl+right" => TuiEvent::CtrlRight,
        // ref `keybindings.ts:231`：`app.tree.filter.cycleBackward` 默认 `shift+ctrl+o`。
        "ctrl+shift+o" => TuiEvent::CtrlShiftO,
        "ctrl+l" => TuiEvent::CtrlL,
        "ctrl+c" => TuiEvent::Clear,
        // ref `tui/keybindings.ts`：`cursorLineStart = [home, ctrl+home, ctrl+a]`、`cursorLeft = [left, ctrl+b]` 等
        "ctrl+a" | "home" | "ctrl+home" => TuiEvent::LineStart,
        "ctrl+e" | "end" | "ctrl+end" => TuiEvent::LineEnd,
        "ctrl+k" => TuiEvent::DeleteToLineEnd,
        "ctrl+u" => TuiEvent::DeleteToLineStart,
        // ref `keybindings.ts:124-126` `tui.editor.deleteWordBackward` = ["ctrl+w", "alt+backspace"]
        "ctrl+w" | "alt+backspace" => TuiEvent::DeleteWordBackward,
        // ref `keybindings.ts:143` `tui.input.newLine` = ["shift+enter", "ctrl+j"]；
        // 另 ref 的**编辑器**在 `components/editor.ts:889-894` 还直接认 `\x1b\r`（legacy alt+enter）
        // 与"含 ESC 且含 \r 的多字节序列"为换行 —— 由键位 oracle（`fixtures-editkeys.json`）实测确认：
        // `\x1b\r` 会插入换行而非提交。
        "ctrl+j" | "shift+enter" | "alt+enter" => TuiEvent::Newline,
        "left" | "ctrl+b" => TuiEvent::CursorLeft,
        "right" | "ctrl+f" => TuiEvent::CursorRight,
        "up" => TuiEvent::HistoryPrev,
        "down" => TuiEvent::HistoryNext,
        // ref `packages/tui/src/keybindings.ts:149/151`：`tui.select.pageUp/pageDown`
        // 的默认键就是 `pageUp`/`pageDown`（`session-selector.ts:621-626` 按整页跳）。
        "pageup" | "pageUp" => TuiEvent::PageUp,
        "pagedown" | "pageDown" => TuiEvent::PageDown,
        "delete" => TuiEvent::Delete,
        other => {
            // 单字符（含空格）⇒ Char；其余（f1..f12 / pageUp / insert / alt+* / shift+*）按 ref 忽略
            let _ = other;
            let mut chars = name.chars();
            if let (Some(ch), None) = (chars.next(), chars.next()) {
                TuiEvent::Char(ch)
            } else {
                return None;
            }
        }
    };
    Some(mapped)
}

#[cfg(test)]
mod p76ae_tests {
    use super::{map_key_sequence, TuiEvent};

    /// S3：映射表（表驱动，覆盖 P7-6ab S1.4 全部条目）。
    #[test]
    fn key_sequence_mapping_table() {
        let cases: &[(&str, Option<TuiEvent>)] = &[
            ("a", Some(TuiEvent::Char('a'))),
            (" ", Some(TuiEvent::Char(' '))),
            ("\r", Some(TuiEvent::Enter)),
            ("\x7f", Some(TuiEvent::Backspace)),
            ("\x1b", Some(TuiEvent::Escape)),
            ("\x04", Some(TuiEvent::CtrlD)),
            ("\x03", Some(TuiEvent::Clear)),
            ("\x01", Some(TuiEvent::LineStart)),
            ("\x05", Some(TuiEvent::LineEnd)),
            ("\x0b", Some(TuiEvent::DeleteToLineEnd)),
            ("\x15", Some(TuiEvent::DeleteToLineStart)),
            ("\x17", Some(TuiEvent::DeleteWordBackward)),
            ("\x0a", Some(TuiEvent::Newline)), // 绑定优先级：newLine(ctrl+j) 先于 submit(enter)
            ("\x1b[D", Some(TuiEvent::CursorLeft)),
            ("\x1b[C", Some(TuiEvent::CursorRight)),
            ("\x1b[A", Some(TuiEvent::HistoryPrev)),
            ("\x1b[B", Some(TuiEvent::HistoryNext)),
            ("\x1b[H", Some(TuiEvent::LineStart)),
            ("\x1b[F", Some(TuiEvent::LineEnd)),
            ("\x1b[3~", Some(TuiEvent::Delete)),
            // ref 未绑定或本批未接线 ⇒ 忽略
            ("\t", Some(TuiEvent::Tab)), // P7-6an/C：Tab 现在映射为补全应用
            // P9-1-A：`tui.select.pageUp` 的默认键就是 pageUp
            // （`packages/tui/src/keybindings.ts:149`），原先按"未绑定"忽略是错的。
            ("\x1b[5~", Some(TuiEvent::PageUp)),
            ("\x1b[6~", Some(TuiEvent::PageDown)),
            ("\x1bOP", None),
            ("\x1b[2~", None),
            ("\x1bb", None),
        ];
        for (seq, expected) in cases {
            assert_eq!(&map_key_sequence(seq), expected, "序列 {seq:?} 映射不符");
        }
    }
}

#[cfg(test)]
mod p76ak_tests {
    use super::{map_key_sequence, TuiEvent};

    /// P7-6ak/C1：**未知序列必须返回 `None`**（不进入编辑器）——
    /// 这是 `7;1u` 泄漏在切默认后不再复发的真正闸门（协商层透传也不会污染屏幕）。
    #[test]
    fn unknown_sequences_never_reach_the_editor() {
        let cases = [
            "\u{1b}[?1;1u",                      // kitty flags 应答样
            "\u{1b}[?62;c",                      // DA1 应答
            "\u{1b}]11;rgb:1111/2222/3333\u{7}", // OSC 11 应答
            "\u{1b}[?997;1n",                    // DSR 997 应答
            "\u{1b}[?0u",                        // kitty flags=0 应答
        ];
        for seq in cases {
            assert_eq!(
                map_key_sequence(seq),
                None,
                "序列 {seq:?} 不应映射为编辑器动作"
            );
        }
        // 对照：裸文本（用户真能敲出来）仍按字符进编辑器——泄漏防护靠的是"序列以 ESC 开头"这条，
        // 而不是"文本里出现 7;1u 就丢弃"。
        assert_eq!(map_key_sequence("7;1u"), Some(TuiEvent::Char('7')));
        assert!(map_key_sequence("a").is_some());
        assert!(map_key_sequence("\r").is_some());
    }
}

#[cfg(test)]
mod p76al_tests {
    use super::TuiEvent;
    use pi_tui::input::is_release;

    /// C：**kitty 激活态的 press/release 必须只处理 press**（ref `tui.ts:1073-1076`）。
    /// Alacritty 在 kitty 协议下发 `\x1b[<cp>;<mod>:1u`（press）与 `:3u`（release）成对出现。
    #[test]
    fn kitty_release_is_dropped_and_press_mapped() {
        let press = "\u{1b}[47;1:1u"; // "/" press
        let release = "\u{1b}[47;1:3u"; // "/" release
        assert!(!is_release(press), "press 不应被判为 release");
        assert!(is_release(release), "release 必须被判为 release");
        assert_eq!(super::map_key_sequence(press), Some(TuiEvent::Char('/')));
        // 粘贴内容里的 `:3F` 样字节不算 release（ref `keys.ts:527-534` 的排除规则）
        assert!(!is_release("\u{1b}[200~90:62:3F:A5\u{1b}[201~"));
    }

    /// C：kitty 形态的换行/删词键必须映射到编辑器动作（ref `keybindings.ts:143`、`:124-126`）。
    #[test]
    fn kitty_forms_map_to_editor_actions() {
        // shift+enter = `\x1b[13;2u`；ctrl+w = `\x1b[119;5u`；alt+backspace = `\x1b[127;3u`
        assert_eq!(
            super::map_key_sequence("\u{1b}[13;2u"),
            Some(TuiEvent::Newline)
        );
        assert_eq!(
            super::map_key_sequence("\u{1b}[119;5u"),
            Some(TuiEvent::DeleteWordBackward)
        );
        assert_eq!(
            super::map_key_sequence("\u{1b}[127;3u"),
            Some(TuiEvent::DeleteWordBackward)
        );
        // alt+enter：ref 的**编辑器**在 `editor.ts:889-894` 直接认 `\x1b\r` 为换行（键位 oracle 实测），
        // 所以 kitty 形态 `\x1b[13;3u` 也必须是换行，而不是"无动作"。
        assert_eq!(
            super::map_key_sequence("\u{1b}[13;3u"),
            Some(TuiEvent::Newline)
        );
    }
}

#[cfg(test)]
mod p76an_tests {
    use super::{TuiEvent, TuiEventSink};
    use pi_core::agent::{AgentEvent, EventSink};

    fn sink() -> (TuiEventSink, std::sync::mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let system = pi_ai::message::Message::System {
            content: pi_ai::message::UserContent::Text(String::new()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        };
        (TuiEventSink::new(tx, 128_000, system), rx)
    }

    fn assistant(stop_reason: pi_ai::StopReason, error: Option<&str>) -> pi_ai::AssistantMessage {
        pi_ai::AssistantMessage {
            role: "assistant".to_string(),
            content: Vec::new(),
            api: "openai-completions".to_string(),
            provider: "p".to_string(),
            model: "m".to_string(),
            response_model: None,
            usage: pi_ai::Usage::default(),
            stop_reason,
            error_message: error.map(str::to_string),
            raw_stop_reason: None,
            end_turn: None,
            response_id: None,
            timestamp: 0,
        }
    }

    /// P7-6an/P（真机 5b）：provider 失败必须**在会话区显示错误**，不能静默无反馈。
    /// 依据 ref `modes/interactive/components/assistant-message.ts:196-198`：渲染 `Error: <errorMessage>`。
    #[test]
    fn provider_error_is_surfaced_in_transcript() {
        let (sink, rx) = sink();
        sink.emit(AgentEvent::MessageEnd {
            message: pi_ai::message::Message::Assistant(Box::new(assistant(
                pi_ai::StopReason::Error,
                Some("401 Unauthorized"),
            ))),
        });
        let mut seen = String::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                // 提示是**独立块**（`AssistantNotice`），正文仍走 `AssistantDelta`。
                TuiEvent::AssistantDelta(delta) | TuiEvent::AssistantNotice(delta) => {
                    seen.push_str(&delta);
                    seen.push('\n');
                }
                _other => {}
            }
        }
        assert!(
            seen.contains("Error: 401 Unauthorized"),
            "错误必须在会话区出现，实际收到 {seen:?}"
        );
    }

    /// 截断与中止同样要有可见提示（ref `:182-193`）。
    #[test]
    fn truncation_and_abort_are_surfaced() {
        let (sink, rx) = sink();
        sink.emit(AgentEvent::MessageEnd {
            message: pi_ai::message::Message::Assistant(Box::new(assistant(
                pi_ai::StopReason::Length,
                None,
            ))),
        });
        sink.emit(AgentEvent::MessageEnd {
            message: pi_ai::message::Message::Assistant(Box::new(assistant(
                pi_ai::StopReason::Aborted,
                None,
            ))),
        });
        let mut seen = String::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                TuiEvent::AssistantDelta(delta) | TuiEvent::AssistantNotice(delta) => {
                    seen.push_str(&delta);
                    seen.push('\n');
                }
                _other => {}
            }
        }
        assert!(
            seen.contains("Response was truncated"),
            "截断提示缺失：{seen:?}"
        );
        assert!(seen.contains("Operation aborted"), "中止提示缺失：{seen:?}");
    }
}

#[cfg(test)]
mod base64_tests {
    use super::base64_encode;

    /// OSC 52 需要标准 base64（含 `=` 填充）。
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"Hello world"), "SGVsbG8gd29ybGQ=");
    }
}

#[cfg(test)]
mod p78a_crash_log_tests {
    use super::append_crash_entry;

    /// 崩溃现场必须**追加**写入（保留历史），且带上位置与消息（真机复现靠它定位）。
    #[test]
    fn crash_entry_is_appended_with_location_and_message() {
        let dir = std::env::temp_dir().join(format!("pi-crash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("crash.log");
        let _ = std::fs::remove_file(&path);
        append_crash_entry(&path, "src/a.rs:1:2", "第一次");
        append_crash_entry(&path, "src/b.rs:3:4", "第二次");
        let text = std::fs::read_to_string(&path).expect("读回崩溃日志");
        assert_eq!(text.lines().count(), 2, "两次崩溃应追加两行：{text}");
        assert!(text.contains("src/a.rs:1:2"), "缺位置：{text}");
        assert!(text.contains("第二次"), "缺消息：{text}");
    }

    /// 目录不存在时不得 panic（panic hook 里再 panic 会二次崩溃）。
    #[test]
    fn missing_directory_is_ignored() {
        append_crash_entry(
            std::path::Path::new("/nonexistent-pi-dir/crash.log"),
            "src/a.rs:1:2",
            "x",
        );
    }
}

#[cfg(test)]
mod p78b_replay_tests {
    use super::transcript_from_messages;
    use pi_ai::message::Message;
    use pi_tui::TranscriptItem;

    /// 用 JSON 构造消息（与真实会话文件同形，避免手搓结构体漏字段）。
    fn parse(json: &str) -> Message {
        serde_json::from_str(json).expect("消息 JSON 合法")
    }

    fn user_blocks(text: &str) -> Message {
        parse(&format!(
            r#"{{"role":"user","content":[{{"type":"text","text":"{text}"}}],"timestamp":0}}"#
        ))
    }

    fn user_text(text: &str) -> Message {
        parse(&format!(
            r#"{{"role":"user","content":"{text}","timestamp":0}}"#
        ))
    }

    fn assistant(text: &str) -> Message {
        parse(&format!(
            r#"{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}],
                "api":"openai-completions","provider":"mock","model":"mock-model",
                "usage":{{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,
                "cost":{{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}}}},
                "stopReason":"stop","timestamp":0}}"#
        ))
    }

    /// 真机实测：会话文件里 user 消息的 `content` 是**块数组**
    /// （`[{"type":"text","text":"hello"}]`），`--continue` 时必须重放出来，
    /// 否则续写的会话只看到助手那一条（用户实测"-c 好像没啥用"）。
    #[test]
    fn block_form_user_message_is_replayed() {
        let items = transcript_from_messages(&[user_blocks("hello"), assistant("Hello world")]);
        assert_eq!(items.len(), 2, "两条都应重放：{items:?}");
        assert_eq!(items[0], TranscriptItem::User("hello".to_string()));
        assert_eq!(
            items[1],
            TranscriptItem::Assistant("Hello world".to_string())
        );
    }

    /// 裸字符串形态（golden 里的种子会话）同样要重放。
    #[test]
    fn plain_text_user_message_is_replayed() {
        let items = transcript_from_messages(&[user_text("say hi")]);
        assert_eq!(items, vec![TranscriptItem::User("say hi".to_string())]);
    }

    /// 空文本不产生空条目（否则续写会多出空行）。
    #[test]
    fn empty_messages_are_skipped() {
        let items = transcript_from_messages(&[user_blocks(""), assistant("")]);
        assert!(items.is_empty(), "空内容不应产生条目：{items:?}");
    }
}

#[cfg(test)]
mod p78b_history_tests {
    use super::produced_tail;
    use pi_ai::message::{Message, UserContent};

    fn user(text: &str) -> Message {
        Message::User {
            content: UserContent::Text(text.to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        }
    }

    /// `run_loop` 返回的是**整段上下文**：只允许把初始历史之后的尾部并回历史，
    /// 否则历史每轮翻倍（真机"越往后越慢"的根因）。
    #[test]
    fn only_the_tail_is_merged_back() {
        let produced = vec![
            user("h1"),
            user("h2"),
            user("new-user"),
            user("new-assistant"),
        ];
        let tail = produced_tail(2, &produced);
        assert_eq!(tail.len(), 2, "应只取尾部：{tail:?}");
        assert_eq!(tail[0], user("new-user"));
    }

    /// 没有新产出时不得把历史再并一遍；历史比产出还长时也不能越界 panic。
    #[test]
    fn empty_or_oversized_history_does_not_panic() {
        assert!(produced_tail(3, &[]).is_empty());
        assert!(produced_tail(9, &[user("only")]).is_empty());
    }
}

#[cfg(test)]
mod clamp_thinking_level_tests {
    use super::clamp_thinking_level;

    fn levels(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    /// 模型支持到 `medium`、请求 `high` ⇒ 夹成 `medium`（真机配置：
    /// `defaultThinkingLevel: high` + `openrouter/free`）。
    #[test]
    fn high_clamps_down_to_model_max() {
        let available = levels(&["off", "minimal", "low", "medium"]);
        assert_eq!(clamp_thinking_level(&available, "high"), "medium");
        assert_eq!(clamp_thinking_level(&available, "max"), "medium");
    }

    /// 支持的档位里直接有请求值 ⇒ 原样返回；非推理模型只有 `off` ⇒ 一律 `off`。
    #[test]
    fn supported_levels_pass_through() {
        let available = levels(&["off", "minimal", "low", "medium", "high"]);
        assert_eq!(clamp_thinking_level(&available, "low"), "low");
        assert_eq!(clamp_thinking_level(&levels(&["off"]), "high"), "off");
    }

    /// 请求值不在档位表里（拼错）⇒ ref `models.ts:942-943` 取 `availableLevels[0] ?? "off"`。
    #[test]
    fn unknown_level_falls_back_to_lowest_available() {
        let available = levels(&["medium", "high"]);
        assert_eq!(clamp_thinking_level(&available, "turbo"), "medium");
        assert_eq!(clamp_thinking_level(&[], "high"), "off");
    }

    /// ref `models.ts:945-952`：不可用时**先向上找**，找不到才向下。
    /// 例如支持集合只有 `off`+`high`、请求 `low` ⇒ 取 `high`（不是 `off`）。
    #[test]
    fn unsupported_level_prefers_higher_neighbour() {
        let available = levels(&["off", "high"]);
        assert_eq!(clamp_thinking_level(&available, "low"), "high");
        assert_eq!(clamp_thinking_level(&available, "minimal"), "high");
        // 已在上限之上则只能向下回退。
        assert_eq!(
            clamp_thinking_level(&levels(&["off", "low"]), "high"),
            "low"
        );
    }
}

#[cfg(test)]
mod windows_input_tests {
    use super::{
        map_windows_key, next_switchable_model, FilterOut, ModelChoice, NextModel, PasteBurst,
        RespFilter, TuiEvent,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    /// 真机缺陷复现：key-up 记录（`KeyEventKind::Release`）必须被丢弃，否则每个字符输入两次。
    #[test]
    fn release_events_are_ignored() {
        let mut release = key(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(map_windows_key(release), None);
        assert_eq!(
            map_windows_key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(TuiEvent::Char('a'))
        );
    }

    /// 真机缺陷复现：crossterm 对 Shift+Tab 给的是 `KeyCode::BackTab`（不是 Tab+SHIFT）。
    #[test]
    fn shift_tab_maps_backtab_key_code() {
        assert_eq!(
            map_windows_key(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(TuiEvent::BackTab)
        );
        assert_eq!(
            map_windows_key(key(KeyCode::Tab, KeyModifiers::NONE)),
            Some(TuiEvent::Tab)
        );
    }

    #[test]
    fn ctrl_letters_map_to_actions() {
        assert_eq!(
            map_windows_key(key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(TuiEvent::CtrlD)
        );
        assert_eq!(
            map_windows_key(key(KeyCode::Char('p'), KeyModifiers::CONTROL)),
            Some(TuiEvent::CtrlP)
        );
        assert_eq!(
            map_windows_key(key(KeyCode::Char('l'), KeyModifiers::CONTROL)),
            Some(TuiEvent::CtrlL)
        );
        assert_eq!(
            map_windows_key(key(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            Some(TuiEvent::CtrlO)
        );
    }

    /// 按线程里的处理顺序喂一整串字符，返回真正会发给主循环的事件。
    fn feed_str(filter: &mut RespFilter, text: &str) -> Vec<TuiEvent> {
        let mut out = Vec::new();
        for ch in text.chars() {
            if ch == '\x1b' {
                filter.on_escape();
                continue;
            }
            match filter.feed(ch) {
                FilterOut::Nothing => {}
                FilterOut::Char(ch) => out.push(TuiEvent::Char(ch)),
                FilterOut::Escape => out.push(TuiEvent::Escape),
                FilterOut::EscapeThenChar(ch) => {
                    out.push(TuiEvent::Escape);
                    out.push(TuiEvent::Char(ch));
                }
                FilterOut::Sequence(seq) => {
                    out.extend(seq.chars().map(TuiEvent::Char));
                }
            }
        }
        out
    }

    /// 真机缺陷复现：conhost / 终端的查询应答不能变成编辑器里的字符。
    #[test]
    fn terminal_replies_are_swallowed() {
        for reply in [
            "\x1b[?1;0c",                       // conhost 的 DA1 应答
            "\x1b[?1;2c",                       // Windows Terminal 的 DA1 应答
            "\x1b[?1u",                         // kitty 键盘协议应答
            "\x1b]11;rgb:0000/0000/0000\x07",   // OSC 11 背景色应答（BEL 结尾）
            "\x1b]11;rgb:ffff/ffff/ffff\x1b\\", // OSC 11（ST 结尾）
            "\x1bOP",                           // SS3（F1）
        ] {
            let mut filter = RespFilter::default();
            let out = feed_str(&mut filter, reply);
            assert!(out.is_empty(), "{reply:?} 被当成输入：{out:?}");
            assert!(!filter.take_pending_escape(), "{reply:?} 之后仍挂着 Esc");
        }
    }

    /// 过滤不能误伤真实按键：单独的 Esc、Esc 后跟普通字符、普通输入。
    #[test]
    fn human_input_survives_the_filter() {
        let mut filter = RespFilter::default();
        assert!(feed_str(&mut filter, "\x1b").is_empty());
        assert!(filter.take_pending_escape());
        assert_eq!(
            feed_str(&mut filter, "\x1bq"),
            vec![TuiEvent::Escape, TuiEvent::Char('q')]
        );
        assert_eq!(
            feed_str(&mut filter, "ab"),
            vec![TuiEvent::Char('a'), TuiEvent::Char('b')]
        );
    }

    fn choices(items: &[(&str, &str)]) -> Vec<ModelChoice> {
        items
            .iter()
            .map(|(provider, id)| ModelChoice {
                provider: (*provider).to_string(),
                id: (*id).to_string(),
            })
            .collect()
    }

    /// 真机缺陷复现：解析失败的 `enabledModels` 项要跳过，否则 Ctrl+P 永久卡在同一条错误上。
    #[test]
    fn ctrl_p_skips_unresolvable_models() {
        let list = choices(&[
            ("openrouter", "openrouter/free"),
            ("command-code", "inclusionai/ling-3.1-flash:free"),
            ("opencode-go", "deepseek-v4.1-flash"),
        ]);
        let next = next_switchable_model(&list, "openrouter/free", "openrouter", |choice| {
            if choice.id.ends_with(":free") {
                Err(format!(
                    "provider {} 下没有名为 {} 的模型",
                    choice.provider, choice.id
                ))
            } else {
                Ok(choice.id.clone())
            }
        });
        match next {
            NextModel::Chosen(choice, built) => {
                assert_eq!(choice.provider, "opencode-go");
                assert_eq!(built, "deepseek-v4.1-flash");
            }
            other => panic!("应跳过解析失败的项，实际：{other:?}"),
        }
    }

    #[test]
    fn ctrl_p_wraps_around_the_list() {
        let list = choices(&[("a", "one"), ("b", "two")]);
        let next = next_switchable_model(&list, "two", "b", |choice| Ok(choice.id.clone()));
        assert_eq!(
            next,
            NextModel::Chosen(
                ModelChoice {
                    provider: "a".to_string(),
                    id: "one".to_string()
                },
                "one".to_string()
            )
        );
    }

    #[test]
    fn ctrl_p_reports_only_one_and_all_failed() {
        let single = choices(&[("a", "one")]);
        assert_eq!(
            next_switchable_model(&single, "one", "a", |choice| Ok(choice.id.clone())),
            NextModel::OnlyOne
        );
        let list = choices(&[("a", "one"), ("b", "two")]);
        match next_switchable_model::<String>(&list, "one", "a", |_choice| Err("boom".to_string()))
        {
            NextModel::NoneSwitchable(error) => assert_eq!(error, "boom"),
            other => panic!("应报全部失败，实际：{other:?}"),
        }
    }

    /// 把字符按固定间隔逐个喂进突发缓冲，返回"喂完后的时刻"。
    fn feed(
        burst: &mut PasteBurst,
        text: &str,
        start: u64,
        gap: u64,
        events: &mut Vec<TuiEvent>,
    ) -> u64 {
        let mut now = start;
        for ch in text.chars() {
            if let Some(previous) = burst.on_char(ch, now) {
                events.extend(previous);
            }
            now += gap;
        }
        now
    }

    /// 事件流里还原出来的文本（`Char` 与 `Paste` 拼接），并断言其中没有提交（`Enter`）。
    fn pasted_text(events: &[TuiEvent]) -> String {
        let mut text = String::new();
        for event in events {
            match event {
                TuiEvent::Char(ch) => text.push(*ch),
                TuiEvent::Paste(body) => text.push_str(body),
                TuiEvent::Enter => panic!("粘贴过程中不应提交：{events:?}"),
                _other => {}
            }
        }
        text
    }

    /// 真机缺陷复现：conhost 的 Shift+Insert 把 `L1⏎L2⏎L3⏎` 拆成字符 + 回车送来，
    /// 旧实现第一行的回车就提交，整段粘贴变成多次对话 ⇒ 现在必须**不发提交**、内容完整。
    #[test]
    fn multi_line_paste_never_submits() {
        let mut burst = PasteBurst::default();
        let mut events = Vec::new();
        let mut now = feed(&mut burst, "L1", 1000, 1, &mut events);
        burst.on_enter(true, now);
        now = feed(&mut burst, "L2", now + 1, 1, &mut events);
        burst.on_enter(true, now);
        now = feed(&mut burst, "L3", now + 1, 1, &mut events);
        burst.on_enter(true, now);
        events.extend(
            burst
                .tick(now + PasteBurst::LOOKAHEAD_MS + 1)
                .unwrap_or_default(),
        );
        assert_eq!(pasted_text(&events), "L1\nL2\nL3\n");
    }

    /// 粘贴被 conhost 分块送达（块间间隔大于字符窗口）时，回车必须走回看窗口变成换行。
    #[test]
    fn chunked_paste_with_large_gaps_still_does_not_submit() {
        let mut burst = PasteBurst::default();
        let mut events = Vec::new();
        let mut now = feed(&mut burst, "L1", 0, 20, &mut events);
        burst.on_enter(true, now);
        now = feed(&mut burst, "L2", now + 20, 20, &mut events);
        burst.on_enter(true, now);
        now = feed(&mut burst, "L3", now + 20, 20, &mut events);
        burst.on_enter(true, now);
        events.extend(
            burst
                .tick(now + PasteBurst::LOOKAHEAD_MS + 1)
                .unwrap_or_default(),
        );
        assert_eq!(pasted_text(&events), "L1\nL2\nL3\n");
    }

    /// 正常打字（间隔 ≳ 60 ms）不被当成粘贴；随后的回车单独提交。
    #[test]
    fn typing_then_enter_submits() {
        let mut burst = PasteBurst::default();
        let mut events = Vec::new();
        let now = feed(&mut burst, "hi", 0, 60, &mut events);
        events.extend(burst.tick(now).unwrap_or_default());
        burst.on_enter(false, now + 60);
        events.extend(
            burst
                .tick(now + 60 + PasteBurst::LOOKAHEAD_MS)
                .unwrap_or_default(),
        );
        assert_eq!(
            events,
            vec![TuiEvent::Char('h'), TuiEvent::Char('i'), TuiEvent::Enter]
        );
    }

    /// 缓冲为空时的回车 = 用户提交。
    #[test]
    fn bare_enter_submits() {
        let mut burst = PasteBurst::default();
        burst.on_enter(false, 10);
        assert_eq!(
            burst.tick(10 + PasteBurst::LOOKAHEAD_MS),
            Some(vec![TuiEvent::Enter])
        );
    }

    /// 单行粘贴带末尾换行（字符后紧跟回车）⇒ 换行并进粘贴，不提交。
    #[test]
    fn single_line_paste_with_newline_inserts_it() {
        let mut burst = PasteBurst::default();
        let mut events = Vec::new();
        let now = feed(&mut burst, "abc", 0, 1, &mut events);
        burst.on_enter(true, now);
        events.extend(
            burst
                .tick(now + PasteBurst::WINDOW_MS + 1)
                .unwrap_or_default(),
        );
        assert_eq!(pasted_text(&events), "abc\n");
    }

    /// 超过窗口间隔的两段字符分别发出（不会跨段拼成一次粘贴）。
    #[test]
    fn gap_splits_bursts() {
        let mut burst = PasteBurst::default();
        assert!(burst.on_char('a', 0).is_none());
        let flushed = burst
            .on_char('b', PasteBurst::WINDOW_MS + 1)
            .expect("间隔超过窗口应先把上一段冲掉");
        assert_eq!(flushed, vec![TuiEvent::Char('a')]);
    }

    /// 无换行的超大粘贴被上限截断（不会无限积累）。
    #[test]
    fn huge_single_line_paste_is_capped() {
        let mut burst = PasteBurst::default();
        let mut emitted = Vec::new();
        for index in 0..(PasteBurst::MAX_CHARS + 5) {
            if let Some(previous) = burst.on_char('x', index as u64) {
                emitted.extend(previous);
            }
        }
        let flushed_chars: usize = emitted
            .iter()
            .map(|event| match event {
                TuiEvent::Paste(text) => text.chars().count(),
                TuiEvent::Char(_) => 1,
                _other => 0,
            })
            .sum();
        assert!(
            flushed_chars >= PasteBurst::MAX_CHARS,
            "到达上限应冲出一批，实际 {} 个字符 / {} 个事件",
            flushed_chars,
            emitted.len()
        );
    }

    #[cfg(test)]
    mod win_input_tests {
        use crate::tui::WindowsInput;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        use pi_tui::TuiEvent;

        /// 合成一个按键（Press）。
        fn press(code: KeyCode) -> KeyEvent {
            KeyEvent::new(code, KeyModifiers::NONE)
        }

        fn press_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
            KeyEvent::new(code, modifiers)
        }

        /// 松开（Release）——真机日志里每个字符都带一条，必须被忽略。
        fn release(code: KeyCode) -> KeyEvent {
            KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release)
        }

        /// 一次粘贴：每字符 Press+Release，间隔 1 ms；换行是 `Ctrl+Enter`（ConPTY 的实际形态）。
        fn paste_events(text: &str, start_ms: u64) -> Vec<(KeyEvent, u64)> {
            let mut out = Vec::new();
            let mut at = start_ms;
            for ch in text.chars() {
                if ch == '\n' {
                    out.push((press_mod(KeyCode::Enter, KeyModifiers::CONTROL), at));
                    out.push((release(KeyCode::Enter), at + 1));
                } else {
                    out.push((press(KeyCode::Char(ch)), at));
                    out.push((release(KeyCode::Char(ch)), at + 1));
                }
                at += 2;
            }
            out
        }

        /// `ESC[200~` / `ESC[201~` 被 ConPTY 拆成按键后的形态。
        fn marker(tail: &str, start_ms: u64) -> Vec<(KeyEvent, u64)> {
            let mut out = vec![(press(KeyCode::Esc), start_ms)];
            for (offset, ch) in tail.chars().enumerate() {
                out.push((press(KeyCode::Char(ch)), start_ms + 1 + offset as u64));
            }
            out
        }

        fn run(events: &[(KeyEvent, u64)], tick_until: u64) -> Vec<TuiEvent> {
            let mut machine = WindowsInput::default();
            let mut out = Vec::new();
            for (key, at) in events {
                out.extend(machine.on_key(*key, *at));
            }
            let mut at = events.last().map_or(0, |(_, at)| *at) + 1;
            while at <= tick_until {
                out.extend(machine.tick(at));
                at += 10;
            }
            out
        }

        fn pastes(events: &[TuiEvent]) -> Vec<String> {
            events
                .iter()
                .filter_map(|event| match event {
                    TuiEvent::Paste(text) => Some(text.clone()),
                    _other => None,
                })
                .collect()
        }

        /// 是否出现过"提交"——`TuiAction::Submit` 由 `apply()` 产生，这里用 Enter 事件
        /// 是否**在粘贴之外**落地来近似：粘贴期间的换行必须是 `Newline` 而不是 `Enter`。
        fn submits(events: &[TuiEvent]) -> usize {
            events
                .iter()
                .filter(|event| matches!(event, TuiEvent::Enter))
                .count()
        }

        fn escapes(events: &[TuiEvent]) -> usize {
            events
                .iter()
                .filter(|event| matches!(event, TuiEvent::Escape))
                .count()
        }

        #[test]
        fn plain_burst_without_markers_becomes_one_paste_and_never_submits() {
            let mut events = paste_events("aa\nbb\ncc\n", 0);
            // 真机日志最后一条是 Shift+Insert 的松开
            events.push((release(KeyCode::Insert), 40));
            let out = run(&events, 400);
            assert_eq!(
                pastes(&out),
                vec!["aa\nbb\ncc\n".to_string()],
                "整段合成一次粘贴"
            );
            assert_eq!(submits(&out), 0, "粘贴不得触发提交");
        }

        #[test]
        fn trailing_marker_residue_is_swallowed() {
            // 只有结尾残留：正文后紧跟被拆开的 `ESC[201~`
            let mut events = paste_events("he\nllo", 0);
            events.extend(marker("[201~", 12));
            events.push((release(KeyCode::Insert), 20));
            let out = run(&events, 400);
            let body = pastes(&out).join("");
            assert_eq!(body, "he\nllo", "正文完整");
            assert!(!body.contains("[201~"), "残留标记不得泄漏进正文");
            assert_eq!(escapes(&out), 0, "标记里的 Esc 不能当成用户按 Esc");
            assert_eq!(submits(&out), 0);
        }

        #[test]
        fn full_markers_become_single_paste_without_residue() {
            let mut events = marker("[200~", 0);
            events.extend(paste_events("hi\nthere", 8));
            events.extend(marker("[201~", 40));
            let out = run(&events, 400);
            assert_eq!(pastes(&out), vec!["hi\nthere".to_string()]);
            assert_eq!(escapes(&out), 0);
            assert_eq!(submits(&out), 0);
        }

        #[test]
        fn split_markers_still_reassemble() {
            // 起始标记被拆到两次读入里：`Esc [` 一批、`200~` 另外几批
            let mut events = vec![(press(KeyCode::Esc), 0), (press(KeyCode::Char('[')), 2)];
            events.extend(marker("200~", 4)[1..].to_vec());
            events.extend(paste_events("x\ny", 12));
            events.extend(marker("[201~", 30));
            let out = run(&events, 400);
            assert_eq!(pastes(&out), vec!["x\ny".to_string()]);
            assert_eq!(submits(&out), 0);
        }

        #[test]
        fn leading_stray_enter_is_absorbed_by_the_burst() {
            // ConPTY 启动时会先送一个 Ctrl+Enter 残留；紧接着就是粘贴正文
            let mut events = vec![(press_mod(KeyCode::Enter, KeyModifiers::CONTROL), 0)];
            events.extend(paste_events("aa\nbb", 3));
            let out = run(&events, 400);
            assert_eq!(pastes(&out), vec!["aa\nbb".to_string()]);
            assert_eq!(submits(&out), 0, "残留回车不得提交");
        }

        #[test]
        fn isolated_escape_still_works() {
            // 单独按 Esc（没有 `[` 跟随）：暂扣 40 ms 后必须放行，且只放行一次
            let events = vec![(press(KeyCode::Esc), 0), (release(KeyCode::Esc), 2)];
            let out = run(&events, 120);
            assert_eq!(escapes(&out), 1);
        }

        #[test]
        fn ctrl_enter_inserts_newline_like_ctrl_j() {
            // ref `keybindings.ts:143`：newLine = ["shift+enter", "ctrl+j"]；
            // ConPTY 用 Ctrl+Enter 表示粘贴换行 ⇒ 必须映射为 Newline 而不是 Enter
            let mut machine = WindowsInput::default();
            // 与提交流程一致：先挂起，`LOOKAHEAD` 之后才裁决
            assert!(machine
                .on_key(press_mod(KeyCode::Enter, KeyModifiers::CONTROL), 0)
                .is_empty());
            let out = machine.tick(100);
            assert_eq!(out, vec![TuiEvent::Newline], "Ctrl+Enter 必须插入换行");
        }
    }
}

#[cfg(test)]
mod write_frame_tests {
    use super::write_frame;

    /// 记录每次 `write` 调用的假 writer（不真写）。
    #[derive(Default)]
    struct Recorder {
        calls: Vec<String>,
        flushed: bool,
    }

    impl std::io::Write for Recorder {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.calls.push(String::from_utf8_lossy(buf).to_string());
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushed = true;
            Ok(())
        }
    }

    /// 不变量（P8e-18 起）：**除最后一次外，每次写入都必须以 `\n` 结尾，且不超过 64 KiB**。
    /// 真机上 ConPTY 会把"写入末尾的不完整转义序列"丢掉、把尾巴当文本显示
    ///（`\x1b[0m` → `0m`、`\x1b]8;;\x07` → `;;`），所以切点不能落在行内；
    /// 分块上限来自 Windows 控制台句柄的单次写入长度限制。
    #[test]
    fn frame_writes_split_only_after_newlines() {
        // 3 万行 × 5 字节 ≈ 150 KiB ⇒ 必须落成多块，且每块都 ≤ 64 KiB。
        let mut payload = String::new();
        for index in 0..30_000 {
            payload.push_str(&format!("L{index}\r\n"));
        }
        let mut out = Recorder::default();
        write_frame(&mut out, &payload).expect("写入");
        assert!(out.flushed, "必须 flush");
        assert!(out.calls.len() > 1, "大帧必须分块");
        for call in &out.calls[..out.calls.len() - 1] {
            assert!(call.ends_with('\n'), "非末次写入未以换行结尾");
            assert!(
                call.len() <= super::WRITE_CHUNK,
                "块超过上限：{} 字节",
                call.len()
            );
        }
        assert_eq!(out.calls.concat(), payload, "拼回的字节必须与输入一致");
    }

    /// 行边界优先于块上限：单行超过 64 KiB 时只能在行尾切（不允许从行中间切开）。
    #[test]
    fn single_long_line_is_not_split() {
        let long = format!("{}\r\n", "x".repeat(super::WRITE_CHUNK + 100));
        let payload = format!("{long}{long}");
        let mut out = Recorder::default();
        write_frame(&mut out, &payload).expect("写入");
        assert_eq!(
            out.calls.len(),
            2,
            "应按行切开，实际 {} 次",
            out.calls.len()
        );
        assert_eq!(out.calls.concat(), payload);
    }

    /// 没有换行的短帧（首帧之外的纯光标定位字节）也要原样写出并 flush。
    #[test]
    fn frame_without_newline_is_one_write() {
        let mut out = Recorder::default();
        write_frame(&mut out, "\u{1b}[?2026l").expect("写入");
        assert_eq!(out.calls, vec!["\u{1b}[?2026l".to_string()]);
        assert!(out.flushed);
    }
}

#[cfg(test)]
mod output_dump_tests {
    use super::dump_output;
    use pi_tui::diff::DiffState;

    /// 输出转储必须把 ESC/CR/LF 转义成可读文本，一帧一条记录头；否则排查时看不出
    /// 到底哪个字节序列被 PTY 吃掉了。（`PI_TUI_OUTPUT_DUMP` 只在真机排查时开。）
    #[test]
    fn output_dump_escapes_control_bytes() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("out.log");
        let mut diff = DiffState::new();
        diff.hardware_cursor_row = 4;
        dump_output(
            &path,
            80,
            24,
            &diff,
            "\u{1b}[2Kpi v0.87.1\u{1b}[0m\u{1b}]8;;\u{7}\r\nnext",
        );
        let text = std::fs::read_to_string(&path).expect("读取转储");
        assert!(text.contains("size=80x24"), "{text}");
        assert!(text.contains("bytes="), "{text}");
        assert!(!text.contains('\u{1b}'), "转储里不应留控制字节：{text:?}");
        assert!(
            text.contains("\\e[2Kpi v0.87.1\\e[0m\\e]8;;\\a\\r\\n\nnext"),
            "{text:?}"
        );
    }
}

#[cfg(test)]
mod cursor_dump_tests {
    use super::{format_cursor_line, TuiState};
    use pi_tui::diff::DiffState;

    /// V6 的转储行字段固定：帧号、尺寸、硬件光标行列、输入长度、忙/档位。
    #[test]
    fn cursor_dump_line_is_stable() {
        let mut diff = DiffState::new();
        diff.hardware_cursor_row = 7;
        diff.cursor_col = 3;
        let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
        state.input = "hi".to_string();
        state.thinking_level = "high".to_string();
        let line = format_cursor_line(12, 345, 100, 30, &diff, &state);
        assert_eq!(
            line,
            "frame=12 ms=345 size=100x30 hw_row=7 col=3 input=2 cursor=0 busy=false thinking=high"
        );
    }
}

#[cfg(test)]
mod q3_wezterm_paste_tests {
    use super::{PasteBurst, TuiEvent};

    /// W-Q3：WezTerm 的 Shift+Insert 把 1200 字符的单行粘贴按**逐键**送来（实测每个字符
    /// Press+Release、间隔 ≲ 1 ms、没有 `ESC[200~`）。旧实现只在出现换行时才当粘贴，
    /// 于是这 1200 个字符被逐个插入，`editor.ts:1296-1312` 的 ">1000 字符折叠" 永远不触发。
    #[test]
    fn long_single_line_burst_becomes_one_paste() {
        let mut burst = PasteBurst::default();
        let mut events = Vec::new();
        for index in 0..1200u64 {
            if let Some(previous) = burst.on_char('x', index) {
                events.extend(previous);
            }
        }
        // 大突发（≥ LARGE_BURST）的空闲窗口是 200 ms，等过它才冲掉
        events.extend(
            burst
                .tick(1200 + PasteBurst::LARGE_BURST_IDLE_MS + 1)
                .unwrap_or_default(),
        );
        assert_eq!(events.len(), 1, "应折叠成一次粘贴：{:?}", events.len());
        match &events[0] {
            TuiEvent::Paste(text) => assert_eq!(text.chars().count(), 1200),
            other => panic!("应为 Paste，实际：{other:?}"),
        }
    }

    /// 多行（3 行合成文本）按逐键 + Ctrl+Enter 换行的真实形状 ⇒ 合并为一次 Paste，且不提交。
    #[test]
    fn wezterm_multiline_keystream_becomes_paste_without_submit() {
        let mut burst = PasteBurst::default();
        let mut events = Vec::new();
        let mut now = 0u64;
        for line in ["SYN-01", "SYN-02", "SYN-03"] {
            for ch in line.chars() {
                if let Some(previous) = burst.on_char(ch, now) {
                    events.extend(previous);
                }
                now += 1;
            }
            burst.on_enter(true, now); // ConPTY 把粘贴里的换行送成 Ctrl+Enter
            now += 1;
        }
        events.extend(
            burst
                .tick(now + PasteBurst::LOOKAHEAD_MS + 1)
                .unwrap_or_default(),
        );
        assert!(
            !events.iter().any(|event| matches!(event, TuiEvent::Enter)),
            "粘贴不应触发提交：{events:?}"
        );
        assert_eq!(events.len(), 1, "{events:?}");
    }
}

#[cfg(test)]
mod q3_newline_binding_tests {
    use super::map_windows_key;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pi_tui::TuiEvent;

    /// ref `packages/tui/src/keybindings.ts:143`：`tui.input.newLine = ["shift+enter", "ctrl+j"]`。
    /// 另外 ConPTY 把粘贴里的换行编码成 **Ctrl+Enter**，也必须走换行分支。
    #[test]
    fn newline_bindings_match_ref() {
        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(map_windows_key(shift_enter), Some(TuiEvent::Newline));
        let ctrl_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
        assert_eq!(map_windows_key(ctrl_enter), Some(TuiEvent::Newline));
        let ctrl_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(map_windows_key(ctrl_j), Some(TuiEvent::Newline));
        let plain_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(map_windows_key(plain_enter), Some(TuiEvent::Enter));
    }
}

#[cfg(test)]
mod hardware_cursor_tests {
    use super::resolve_show_hardware_cursor;

    /// ref `core/settings-manager.ts:1369-1370`：`settings.showHardwareCursor ?? env === "1"`。
    #[test]
    fn settings_wins_then_env_then_default_off() {
        assert!(resolve_show_hardware_cursor(Some(true), Some("0")));
        assert!(!resolve_show_hardware_cursor(Some(false), Some("1")));
        assert!(resolve_show_hardware_cursor(None, Some("1")));
        assert!(!resolve_show_hardware_cursor(None, Some("0")));
        assert!(!resolve_show_hardware_cursor(None, None));
    }
}

#[cfg(test)]
mod tab_in_paste_tests {
    use super::{map_windows_key, WindowsInput};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pi_tui::TuiEvent;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    /// 键流粘贴（conhost Shift+Insert）里的 tab：攒批期间应当作正文里的 '\t'，而不是 Tab 键。
    #[test]
    fn tab_inside_keystream_paste_stays_in_text() {
        let mut input = WindowsInput::default();
        let mut events = Vec::new();
        let mut now = 0u64;
        for ch in "AAA".chars() {
            events.extend(input.on_key(press(KeyCode::Char(ch), KeyModifiers::NONE), now));
            now += 1;
        }
        input.on_key(press(KeyCode::Enter, KeyModifiers::NONE), now); // 换行（粘贴里）
        now += 1;
        events.extend(input.on_key(press(KeyCode::Tab, KeyModifiers::NONE), now));
        now += 1;
        for ch in "XYZ".chars() {
            events.extend(input.on_key(press(KeyCode::Char(ch), KeyModifiers::NONE), now));
            now += 1;
        }
        events.extend(input.tick(now + 10_000));
        let pasted: String = events
            .iter()
            .map(|event| match event {
                TuiEvent::Paste(text) => text.clone(),
                TuiEvent::Char(ch) => ch.to_string(),
                other => panic!("不应出现 {other:?}"),
            })
            .collect();
        assert_eq!(pasted, "AAA\n\tXYZ", "tab 应保留在粘贴文本里");
    }

    /// 空闲时按 Tab 仍是补全键（不并入任何粘贴）。
    #[test]
    fn tab_outside_paste_is_still_the_tab_key() {
        let mut input = WindowsInput::default();
        let events = input.on_key(press(KeyCode::Tab, KeyModifiers::NONE), 0);
        assert_eq!(events, vec![TuiEvent::Tab]);
        assert_eq!(
            map_windows_key(press(KeyCode::Tab, KeyModifiers::NONE)),
            Some(TuiEvent::Tab)
        );
    }
}

#[cfg(test)]
mod marker_split_tests {
    use super::WindowsInput;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pi_tui::TuiEvent;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn feed_chars(
        input: &mut WindowsInput,
        text: &str,
        start: u64,
        gap: u64,
    ) -> (Vec<TuiEvent>, u64) {
        let mut events = Vec::new();
        let mut now = start;
        for ch in text.chars() {
            events.extend(input.on_key(press(KeyCode::Char(ch)), now));
            now += gap;
        }
        (events, now)
    }

    /// 用户真机：粘贴结束后 `ESC[201~` 被 ConPTY **分片**送达（分片间隔 > 120 ms 窗口），
    /// 旧实现把已匹配的 `[20` 丢掉、把尾巴 `1~` 当正文插进编辑器。
    #[test]
    fn split_end_marker_must_not_leak_its_tail() {
        let mut input = WindowsInput::default();
        // 粘贴正文（走括号收集）
        let mut events = Vec::new();
        events.extend(input.on_key(press(KeyCode::Esc), 0));
        let (more, now) = feed_chars(&mut input, "[200~ABC", 1, 1);
        events.extend(more);
        // 结束标记：ESC [ 2 0 送达后停 300 ms，再送 1 ~
        events.extend(input.on_key(press(KeyCode::Esc), now));
        let (more, now) = feed_chars(&mut input, "[20", now + 1, 1);
        events.extend(more);
        let (more, _now) = feed_chars(&mut input, "1~", now + 300, 1);
        events.extend(more);
        let text: String = events
            .iter()
            .map(|event| match event {
                TuiEvent::Paste(text) => text.clone(),
                TuiEvent::Char(ch) => ch.to_string(),
                other => format!("<{other:?}>"),
            })
            .collect();
        assert_eq!(text, "ABC", "尾部分片不应落成正文：{text:?}");
    }
}

#[cfg(test)]
mod resp_filter_tests {
    use super::*;

    /// P8e-17 真机缺陷：`ESC[201~` 逐字节到达、且在 CSI 中途被 tick 打断时，
    /// 旧实现把状态重置成 Idle ⇒ `ESC[20` 已被吞掉、`1~` 漏进编辑器。
    #[test]
    fn slow_csi_never_leaks_tail_after_tick() {
        let mut filter = RespFilter::default();
        filter.on_escape();
        assert_eq!(filter.feed('['), FilterOut::Nothing);
        assert!(!filter.take_pending_escape(), "Csi 不应被当成孤立 Esc 放行");
        let mut out = String::new();
        for ch in "201~".chars() {
            assert!(!filter.take_pending_escape());
            match filter.feed(ch) {
                FilterOut::Nothing => {}
                FilterOut::Sequence(seq) => out.push_str(&seq),
                FilterOut::Char(ch) => out.push(ch),
                FilterOut::Escape | FilterOut::EscapeThenChar(_) => {}
            }
        }
        // 合法结果只有两种：整段吞掉（空），或**整段**回放；不允许只吐尾巴。
        assert!(
            out.is_empty() || out == "\u{1b}[201~",
            "只允许整体吞掉或整体回放，实际：{out:?}"
        );
    }

    /// 真正的终端应答仍要吞掉（DA1 / kitty / 光标报告）。
    #[test]
    fn terminal_replies_are_swallowed() {
        for reply in ["\u{1b}[?1;0c", "\u{1b}[?1u", "\u{1b}[>7u", "\u{1b}[12;40R"] {
            let mut filter = RespFilter::default();
            filter.on_escape();
            for (index, ch) in reply.chars().enumerate() {
                let out = filter.feed(ch);
                if index > 0 {
                    assert_eq!(out, FilterOut::Nothing, "{reply:?} 应整体吞掉");
                }
            }
        }
    }

    /// 非应答序列整段回放（不丢字节）。
    #[test]
    fn unknown_csi_is_replayed_whole() {
        let mut filter = RespFilter::default();
        filter.on_escape();
        assert_eq!(filter.feed('['), FilterOut::Nothing);
        let mut out = String::new();
        for ch in "201~".chars() {
            match filter.feed(ch) {
                FilterOut::Nothing => {}
                FilterOut::Sequence(seq) => out.push_str(&seq),
                FilterOut::Char(ch) => out.push(ch),
                FilterOut::Escape | FilterOut::EscapeThenChar(_) => {}
            }
        }
        assert_eq!(out, "\u{1b}[201~", "非应答序列应整段回放");
    }
}

#[cfg(test)]
mod crlf_paste_tests {
    use super::*;

    fn burst_text(chars: &[(char, u64, bool)]) -> String {
        // (字符, 时刻, 是否 LF)
        let mut burst = PasteBurst::default();
        let mut text = String::new();
        for (ch, at, is_lf) in chars {
            let produced = if *ch == '\r' {
                if *is_lf {
                    burst.on_enter(true, *at);
                } else {
                    burst.on_enter(false, *at);
                }
                None
            } else {
                burst.on_char(*ch, *at)
            };
            if let Some(events) = produced {
                for event in events {
                    match event {
                        pi_tui::TuiEvent::Paste(body) => text.push_str(&body),
                        pi_tui::TuiEvent::Char(c) => text.push(c),
                        _ => {}
                    }
                }
            }
        }
        for event in burst.take_all() {
            match event {
                pi_tui::TuiEvent::Paste(body) => text.push_str(&body),
                pi_tui::TuiEvent::Char(c) => text.push(c),
                _ => {}
            }
        }
        text
    }

    /// 一行结尾的 `CR`+`LF`（间隔 2 ms，真机日志）只算一个换行。
    #[test]
    fn crlf_pair_becomes_single_newline() {
        let text = burst_text(&[
            ('a', 0, false),
            ('\r', 2, false),
            ('\r', 4, true),
            ('b', 6, false),
        ]);
        assert_eq!(text, "a\nb", "CRLF 应合并成一个换行：{text:?}");
    }

    /// **空行不能被吞**：`CRLF CRLF` 必须产生两个换行。
    #[test]
    fn consecutive_crlf_preserves_blank_line() {
        let text = burst_text(&[
            ('a', 0, false),
            ('\r', 2, false),
            ('\r', 4, true),
            ('b', 6, false),
            ('\r', 8, false),
            ('\r', 10, true),
            ('c', 12, false),
        ]);
        assert_eq!(text, "a\nb\nc", "{text:?}");
        // 真正的空行：`a CRLF CRLF b`
        let text2 = burst_text(&[
            ('a', 0, false),
            ('\r', 2, false),
            ('\r', 4, true),
            ('\r', 6, false),
            ('\r', 8, true),
            ('b', 10, false),
        ]);
        assert_eq!(text2, "a\n\nb", "空行必须保留：{text2:?}");
    }

    /// 单独的 `CR`（老式换行）仍然算换行。
    #[test]
    fn lone_cr_is_a_newline() {
        let text = burst_text(&[('a', 0, false), ('\r', 2, false), ('b', 10, false)]);
        assert_eq!(text, "a\nb", "{text:?}");
    }

    /// 单独的 `LF`（`Ctrl+Enter`，用户按 ctrl+j）也算换行。
    #[test]
    fn lone_lf_is_a_newline() {
        let text = burst_text(&[('a', 0, false), ('\r', 2, true), ('b', 10, false)]);
        assert_eq!(text, "a\nb", "{text:?}");
    }
}

#[cfg(test)]
mod login_panel_window_tests {
    use super::{login_render, LoginItem, TuiState};

    fn items(n: usize) -> Vec<LoginItem> {
        (0..n)
            .map(|i| LoginItem {
                label: format!("provider-{i:02} (api_key)"),
                id: Some(format!("p{i}")),
                auth_type: crate::login_options::AuthType::ApiKey,
            })
            .collect()
    }

    fn panel_lines(state: &TuiState) -> Vec<String> {
        let (_, lines) = state.panel.as_ref().expect("面板应已打开");
        lines.clone()
    }

    /// 选中项必须出现在可视窗口内，并附带 `  (n/total)`；
    /// 此前把全部 provider 一次铺出来，条目多于屏幕高度时选中项被顶出视图。
    #[test]
    fn selected_item_stays_visible_with_scroll_hint() {
        let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
        let list = items(25);
        login_render(&mut state, &list, 17);
        let lines = panel_lines(&state);
        assert!(
            lines.iter().any(|line| line.contains("provider-17")),
            "选中项必须在窗口内：{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("(18/25)")),
            "需要滚动提示：{lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("provider-00")),
            "窗口应上移，首项不应出现：{lines:?}"
        );
        assert!(
            lines
                .iter()
                .filter(|line| line.contains("provider-"))
                .count()
                == 8,
            "窗口最多 8 项：{lines:?}"
        );
    }

    /// 条目不超过 8 条时不显示滚动提示（ref 同条件）。
    #[test]
    fn short_list_has_no_scroll_hint() {
        let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
        let list = items(3);
        login_render(&mut state, &list, 1);
        let lines = panel_lines(&state);
        assert_eq!(lines.iter().filter(|l| l.contains("provider-")).count(), 3);
        assert!(
            !lines.iter().any(|line| line.contains("(2/3)")),
            "不应出现滚动提示：{lines:?}"
        );
    }
}
