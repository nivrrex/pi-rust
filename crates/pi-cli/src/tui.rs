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
                                output: None,
                                is_error: false,
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

/// 用 `models.json` + 内置目录重新解析并构造指定模型的 provider（`Ctrl+P` 用）。
fn rebuild_provider(
    parts: &AgentParts,
    provider: &str,
    model_id: &str,
) -> Result<std::sync::Arc<dyn pi_ai::Provider>, String> {
    let path = parts.agent_dir.join("models.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("{} 读取失败：{error}", path.display()))?;
    let config = serde_json::from_str::<pi_ai::config::ModelsConfig>(&text)
        .map_err(|error| format!("models.json 解析失败：{error}"))?;
    let catalog = crate::runtime::load_builtin_catalog(&parts.agent_dir);
    let resolved = config
        .resolve_with_catalog(provider, model_id, catalog.as_ref())
        .map_err(|error| format!("{provider}/{model_id} 解析失败：{error}"))?;
    crate::runtime::build_provider(&resolved, None)
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
            SelectorUi::Resume(_, _) => {}
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
            SelectorUi::Resume(_, _) => {}
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
    b"\x1b[?2004h\x1b[>7u\x1b[?u\x1b[c\x1b[?2031h\x1b[?996n\x1b[?25l"
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

pub fn run(mut state: TuiState, parts: Option<std::sync::Arc<AgentParts>>) -> std::io::Result<i32> {
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
    let mut queued_inputs: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    // `--continue` 时把历史消息灌进 `history`，并把可显示的部分放进 transcript
    // （否则续写的会话在 TUI 里是空屏、`/tree` 也看不到历史）。
    // 思考档位集合（ref `ai/src/models.ts:924-933`）：非推理模型只有 `off`，
    // 推理模型是 `EXTENDED_THINKING_LEVELS`（`off/minimal/low/medium/high/xhigh/max`）。
    if let Some(parts_ref) = parts.as_ref() {
        if parts_ref.supports_thinking {
            state.available_thinking_levels =
                ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                    .iter()
                    .map(|level| (*level).to_string())
                    .collect();
            state.thinking_level = parts_ref
                .initial_thinking_level
                .clone()
                .unwrap_or_else(|| "medium".to_string());
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
    // 最近一条 assistant 的 `usage.input`（ref 的上下文占用基数用它，**不是**历史累加）。
    let last_assistant_input = initial
        .iter()
        .rev()
        .find_map(|message| match message {
            pi_ai::message::Message::Assistant(assistant) => Some(assistant.usage.input),
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
    spawn_r3_input_thread(input_tx);

    let mut cols = terminal::size().map(|(c, _)| c as usize).unwrap_or(80);
    let mut rows = terminal::size().map(|(_, r)| r as usize).unwrap_or(24);
    // R2 需要上一帧的屏幕行；R1 忽略它。resize 时必须清空（下一帧整屏重画）。
    let mut previous: Vec<String> = Vec::new();
    // P7-6n/W2：`r3` 的差分状态（R1/R2 不使用）。
    let mut diff_state = pi_tui::diff::DiffState::new();
    // P7-6an/U：`showHardwareCursor`（ref `tui.ts:478` 默认 false，由 settings.json 传入，
    // `cli/startup-ui.ts:89`）。开启后 r3 每帧写 `\x1b[?25h` 显示硬件光标。
    diff_state.show_hardware_cursor = std::env::var("PI_CODING_AGENT_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .and_then(|dir| std::fs::read_to_string(dir.join("settings.json")).ok())
        .and_then(|text| serde_json::from_str::<crate::settings::Settings>(&text).ok())
        .and_then(|settings| settings.show_hardware_cursor)
        .unwrap_or(false);
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
            let window = if parts_ref.context_window >= 1000 {
                format!("{}k", parts_ref.context_window / 1000)
            } else {
                parts_ref.context_window.to_string()
            };
            // ref `agent-session.ts:3858-3902` 的 `estimateContextTokens`：**有 assistant 的 usage 时**
            // 直接用 `calculateContextTokens(usage)`（≈ input，不含 output）当基数；
            // 只有在没有可用 usage 时才退化为逐条估算。
            let percent = if parts_ref.context_window == 0 {
                None
            } else if last_assistant_input > 0 {
                Some(last_assistant_input as f64 / parts_ref.context_window as f64 * 100.0)
            } else {
                let system_tokens =
                    pi_core::agent::compaction::estimate_tokens(&parts_ref.system_message);
                Some(system_tokens as f64 / parts_ref.context_window as f64 * 100.0)
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
                    | TuiEvent::ClearStatus
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
                                | TuiEvent::Clear
                                | TuiEvent::Paste(_)
                                | TuiEvent::Quit
                                | TuiEvent::AssistantNotice(_)
                                | TuiEvent::ClearStatus
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
                                        let window = parts_ref.context_window;
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
                                        let window = if parts.context_window >= 1000 {
                                            format!("{}k", parts.context_window / 1000)
                                        } else {
                                            parts.context_window.to_string()
                                        };
                                        let percent = if parts.context_window == 0 {
                                            None
                                        } else {
                                            Some(
                                                (system_tokens + message_tokens) as f64
                                                    / parts.context_window as f64
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
                                                            parts.context_window / 1000
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
                                    serde_json::Value::String(theme),
                                );
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
                                Ok(provider) => {
                                    let provider_name = pick_provider.clone();
                                    if let Ok(mut guard) = parts_ref.model_state.lock() {
                                        guard.model_id.clone_from(&model_id);
                                        guard.provider_name.clone_from(&pick_provider);
                                        guard.provider = provider;
                                    }
                                    if let Some(writer) = parts_ref.writer.as_ref() {
                                        if let Ok(mut guard) = writer.lock() {
                                            let _ = guard
                                                .append_model_change(&provider_name, &model_id);
                                        }
                                    }
                                    state.model = model_id.clone();
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
                    let next_id = if ids.len() <= 1 {
                        None
                    } else {
                        let index = ids
                            .iter()
                            .position(|choice| {
                                choice.id == current && choice.provider == current_provider
                            })
                            .unwrap_or(0);
                        Some(ids[(index + 1) % ids.len()].clone())
                    };
                    match next_id {
                        None => {
                            let _ = apply(
                                &mut state,
                                TuiEvent::Status("Only one model available".to_string()),
                            );
                        }
                        Some(next_id) => {
                            match rebuild_provider(parts_ref, &next_id.provider, &next_id.id) {
                                Ok(provider) => {
                                    if let Ok(mut guard) = parts_ref.model_state.lock() {
                                        guard.model_id.clone_from(&next_id.id);
                                        guard.provider_name.clone_from(&next_id.provider);
                                        guard.provider = provider;
                                    }
                                    if let Some(writer) = parts_ref.writer.as_ref() {
                                        if let Ok(mut guard) = writer.lock() {
                                            let _ = guard.append_model_change(
                                                &next_id.provider,
                                                &next_id.id,
                                            );
                                        }
                                    }
                                    state.model = next_id.id.clone();
                                    let _ = apply(
                                        &mut state,
                                        TuiEvent::Status(format!("Switched to {}", next_id.id)),
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
            match apply(&mut state, event) {
                TuiAction::Exit => break 'main,
                TuiAction::Submit(text) => {
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
                            let sink = std::sync::Arc::new(TuiEventSink::new(
                                tx.clone(),
                                parts.context_window,
                                parts.system_message.clone(),
                            ));
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
                            let window = if parts.context_window >= 1000 {
                                format!("{}k", parts.context_window / 1000)
                            } else {
                                parts.context_window.to_string()
                            };
                            let percent = if parts.context_window == 0 {
                                None
                            } else {
                                Some(estimate.tokens as f64 / parts.context_window as f64 * 100.0)
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
                    match name.as_str() {
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
                            let (choices, current_id, current_provider) =
                                parts.as_ref().map_or_else(
                                    || {
                                        (
                                            vec![ModelChoice {
                                                provider: "openai-completions".to_string(),
                                                id: "mock-model".to_string(),
                                            }],
                                            "mock-model".to_string(),
                                            "openai-completions".to_string(),
                                        )
                                    },
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
                            }
                            sel.home = std::env::var("HOME").unwrap_or_default();
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
    Ok(0)
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
    {
        {
            // P7-6n：主屏差分渲染（`TuiMainScreen.doRender` 移植）。`render_all` 给**全量**行，
            // 可见窗口由 `diff_render` 的 `height` 决定（不预裁剪）。
            // **不再外包同步序列**：`diff_render` 自己已经写出 `?2026h/…/?2026l`（`tui-main-screen.ts:283/317/494/567`）。
            let lines = pi_tui::render_all_marker_cached(state, cols, rows, render_cache);
            // W1：诊断转储（feature 关闭时这段不参与编译）。
            #[cfg(feature = "diag-frame-dump")]
            {
                static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                diag_dump::dump_frame(seq, cols, rows, &lines, diff_state, 0);
            }
            match pi_tui::diff::diff_render_with_cursor(diff_state, &lines, cols, rows) {
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
    out.write_all(payload.as_bytes())?;
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
}

impl TuiEventSink {
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
        }
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
                    } else if assistant.usage.input + assistant.usage.output > 0 {
                        // 有 usage（T01 的 `↑10 ↓2`）：用量口径 ⇒ 12/128000 = 0.0%。
                        Some(
                            (assistant.usage.input + assistant.usage.output) as f64
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
                        Some(
                            (system_tokens + text_tokens) as f64 / self.context_window as f64
                                * 100.0,
                        )
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
                tool_name, args, ..
            } => Some(TuiEvent::ToolCall {
                name: tool_name,
                summary: tool_summary(&args),
            }),
            AgentEvent::ToolExecutionEnd {
                result, is_error, ..
            } => {
                // 结果正文：把所有 text 块按换行拼接（ref `render-utils.ts` 的
                // `getTextOutput` 同口径），图片块在 TUI 里略过。
                let mut parts: Vec<String> = Vec::new();
                if let Some(blocks) = result.as_array() {
                    for block in blocks {
                        if block.get("type").and_then(|kind| kind.as_str()) == Some("text") {
                            if let Some(text) = block.get("text").and_then(|text| text.as_str()) {
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
                Some(TuiEvent::ToolResult {
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
        assert!(r3.contains("\x1b[?2004h"), "r3 启动应开启括号粘贴：{r3:?}");
        // P7-6ae/S2b：kitty 查询三元组已接线（应答由协商层消费），顺序 2004h → kitty → 25l。
        let paste_at = r3.find("\x1b[?2004h").expect("缺 2004h");
        let kitty_at = r3
            .find("\x1b[>7u\x1b[?u\x1b[c")
            .expect("缺 kitty 查询三元组");
        let hide_at = r3.find("\x1b[?25l").expect("缺 hideCursor");
        assert!(
            paste_at < kitty_at && kitty_at < hide_at,
            "启动顺序应为 2004h → kitty → 25l：{r3:?}"
        );
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
    use super::{r3_exit_bytes, startup_bytes};

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
    #[test]
    fn r3_startup_contains_kitty_triplet() {
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
            ("\x1b[5~", None),
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
