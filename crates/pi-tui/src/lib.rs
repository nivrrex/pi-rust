//! `pi-tui`：全屏 TUI 的**纯逻辑**层（状态 + 事件 → 行文本）。
//!
//! 设计约束（P7-3/W2）：本 crate **不做任何终端 I/O**（不读键盘、不写终端、不动 raw mode），
//! 因此可以单测；终端 I/O 层放在 `pi-cli`。
//!
//! 头部件文案逐条来自 `ref/.../modes/interactive/interactive-mode.ts`：
//! - `:958` `const logo = theme.bold(theme.fg("accent", APP_NAME)) + theme.fg("dim", ` v${this.version}`)`
//!   ⇒ 版本行 ` pi v<version>`（前导空格来自头部件内边距）。
//! - `:985-990` 键位提示行（`hint("app.interrupt","interrupt")` 等，用 `" · "` 连接）；
//! - `:991-994` `Press ctrl+o to show full startup help and loaded resources.`；
//! - `:995-998` `Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.`；
//! - `:1770-1784` `[Context]` 段（每行一个 `formatContextPath` 后的路径）。
//! - 状态行 `components/footer.ts:151-155`：`<pct>%/<window> (auto)`。
//!
//! 未实现（本批范围外，见 `RUNBOOK-8` 与 `notes/reports/P7-3-spec.md`）：完整键位、斜杠命令、
//! 会话树、模型选择器、主题、图片显示等。

#![forbid(unsafe_code)]

// P7-6k/W2：主屏差分渲染的纯函数移植（本批不接线，见模块文档）。
pub mod diff;
// P7-6q/W3：键盘输入层纯函数（本批不接线）。
pub mod input;
// P7-6w/W2：终端协商状态机（纯函数，本批不接线）。
pub mod term_negotiate;
// P7-6y/W3：OSC 11 / DSR 996 的配额状态（纯函数，本批不接线）。
pub mod osc11;
// P7-6af/D：DSR 996/997 彩色方案查询（纯函数）。
pub mod dsr996;
// P7-6r2/W2：/hotkeys 面板的**多宽度常量表**（自动生成，见 `gen-hotkeys-rs.mjs`）。
mod hotkeys_widths;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// 启动屏显示的版本号：**上游 pi 的版本**（`ref/` 是 tag `v0.87.1`；golden-tui 实测为 `pi v0.87.1`），
/// 与 `pi_core::VERSION`（本 crate 的包版本 `0.87.1`，用于 `--version`）**不是一回事**。
pub const DISPLAY_VERSION: &str = "0.87.1";

/// 运行中指示器的动画帧（ref `tui/src/components/loader.ts:13` 的 `DEFAULT_FRAMES`）。
pub const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// 帧间隔（ref `loader.ts:14` 的 `DEFAULT_INTERVAL_MS = 80`）。
pub const SPINNER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(80);

/// TUI 的可变状态（纯数据）。
#[derive(Debug, Clone, PartialEq)]
pub struct TuiState {
    /// 版本号（用于 `pi v<version>`）。
    pub version: String,
    /// 当前工作目录（状态行左侧）。
    pub cwd: String,
    /// 已加载的项目指令文件路径（`[Context]` 段，按加载顺序）。
    pub context_files: Vec<String>,
    /// 编辑器当前内容（可含 `\n` ⇒ 多行；`Ctrl+J` 插入换行，实测见 P7-5/W1）。
    pub input: String,
    /// 光标位置（按**字符**计，而不是字节；宽字符因此天然按字符处理）。
    pub cursor: usize,
    /// 已提交过的输入历史（最近的在最后）。`Up`/`Down` 在其上移动。
    pub history: Vec<String>,
    /// 当前浏览到的历史下标；`None` = 不在浏览态（编辑器里是本轮新输入）。
    pub history_pos: Option<usize>,
    /// `/` 触发的补全弹层是否打开（Esc 关闭）。
    pub completion_open: bool,
    /// 补全菜单的选中项（ref `editor.ts:755-800`：↑/↓ 在菜单打开时移动选中项）。
    pub completion_selected: usize,
    /// 当前打开的命令面板（放置位置 + 已渲染好的行）；`None` 表示没有面板。
    pub panel: Option<(PanelPlacement, Vec<String>)>,
    /// 面板内当前选中项（`/model` 方向键用；0 表示第一项）。
    pub panel_selection: usize,
    /// 折叠的大段粘贴：编号 → 原文（ref `editor.ts:1301-1313` 的 `pastes`）。
    pub pastes: std::collections::BTreeMap<u32, String>,
    /// 粘贴编号计数器（ref `pasteCounter`；只在 `setText` 时重置）。
    pub paste_counter: u32,
    /// 当前思考等级（`off`/`minimal`/`low`/`medium`/`high`/`xhigh`/`max`）。
    pub thinking_level: String,
    /// 当前模型支持的思考等级（非推理模型只有 `off`，ref `ai/src/models.ts:924-933`）。
    pub available_thinking_levels: Vec<String>,
    /// 运行中指示器的当前帧下标（`SPINNER_FRAMES`）。
    pub spinner_frame: usize,
    /// 会话累计输入 token（ref `footer.ts` 的 `usageTotals.input`：**跨回合累加**）。
    pub usage_input: u64,
    /// 会话累计输出 token。
    pub usage_output: u64,
    /// 会话累计费用（美元；ref `footer.ts:143-146`：为 0 时不显示）。
    pub usage_cost: f64,
    /// 底部状态行右侧（如 `0.0%/128k (auto)`）。
    pub status: String,
    /// 状态区第二行右侧显示的模型名（golden 里是 `mock-model`，右侧对齐）。
    pub model: String,
    /// 是否已请求退出。
    pub exited: bool,
    /// 会话区（W3-2）：用户消息、助手回复、工具调用块，按发生顺序。
    pub transcript: Vec<TranscriptItem>,
    /// agent loop 是否正在运行（对应 golden 里的 `Working` 行）。
    pub working: bool,
}

/// 会话区的一项（**纯数据**）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptItem {
    /// 用户消息（golden 里形如 ` say hi`）。
    User(String),
    /// 助手回复（markdown 子集，经 [`render_markdown`] 渲染）。
    Assistant(String),
    /// 分支摘要块（ref `components/branch-summary-message.ts:10-63`）：
    /// ` [branch]` + 空行 + 折叠态 ` Branch summary (ctrl+o to expand)`（展开态另算）。
    BranchSummary {
        /// 摘要正文（折叠态不显示）。
        summary: String,
        /// 是否展开（`ctrl+o` 切换；本实现暂未接键盘）。
        expanded: bool,
    },
    /// 状态通知行（ref `interactive-mode.ts:3695-3716` 的 `showStatus`：`Text(msg, 1, 0)`，
    /// 即前导一个空格；连续同一条时**原地替换**而不是新增一行）。
    Status(String),
    /// 思考块（ref `assistant-message.ts:118-150`；默认显示，按 markdown 渲染）。
    Thinking(String),
    /// 工具调用块（golden/T06 实测：只显示调用行，如 ` read input.txt`；结果正文默认不显示）。
    ToolCall {
        /// 工具名（如 `read`）。
        name: String,
        /// 已格式化的参数摘要（如 `input.txt`）。
        summary: String,
    },
}

impl TuiState {
    /// 推进一帧动画（由事件循环按 [`SPINNER_INTERVAL`] 调用；ref 的 `setInterval` 同款）。
    pub fn tick_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
    }

    /// 构造初始状态（启动屏）。
    #[must_use]
    pub fn new(
        version: impl Into<String>,
        cwd: impl Into<String>,
        status: impl Into<String>,
    ) -> Self {
        Self {
            thinking_level: "off".to_string(),
            available_thinking_levels: vec!["off".to_string()],
            spinner_frame: 0,
            pastes: std::collections::BTreeMap::new(),
            paste_counter: 0,
            usage_input: 0,
            usage_output: 0,
            usage_cost: 0.0,
            version: version.into(),
            cwd: cwd.into(),
            context_files: Vec::new(),
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_pos: None,
            completion_open: false,
            completion_selected: 0,
            panel: None,
            panel_selection: 0,
            status: status.into(),
            model: String::new(),
            exited: false,
            transcript: Vec::new(),
            working: false,
        }
    }
}

/// 输入事件（由终端层翻译成这些语义事件）。
// 不加 Eq：Usage.percent 是 Option<f64>（浮点不实现 Eq）。
#[derive(Debug, Clone, PartialEq)]
pub enum TuiEvent {
    /// 可打印字符。
    Char(char),
    /// 退格。
    Backspace,
    /// `ctrl+o`（ref 里既是 `/tree` 的"过滤循环"，也是主界面的"显示更多"）。
    CtrlO,
    /// `ctrl+t`（`/tree` 的过滤切换；主界面无绑定）。
    CtrlT,
    /// `ctrl+p`（主界面：循环到下一个模型；`/tree` 里无绑定）。
    CtrlP,
    /// `ctrl+shift+o`（`/tree` 的反向过滤循环；ref `keybindings.ts:231`）。
    CtrlShiftO,
    /// `ctrl+s`（`/resume`/`/settings` 里的排序/默认设置键）。
    CtrlS,
    /// `ctrl+n`（`/resume` 的"只看命名会话"；主界面无绑定）。
    CtrlN,
    /// `ctrl+r`（`/resume` 的重命名；主界面无绑定）。
    CtrlR,
    /// `ctrl+x`（`app.message.copy`：复制最后一条助手消息）。
    CtrlX,
    /// `ctrl+←`（`/tree` 的折叠/上移；`/resume` 的翻页）。
    CtrlLeft,
    /// `ctrl+→`（`/tree` 的展开/下移；`/resume` 的翻页）。
    CtrlRight,
    /// `ctrl+l`（`/tree` 的过滤切换；主界面无绑定）。
    CtrlL,
    /// 回车（提交）。
    Enter,
    /// `Ctrl+D`：**空编辑器时退出**（`core/keybindings.ts:95` `app.exit` 的 `description: "Exit when editor is empty"`）。
    CtrlD,
    /// 光标左移（跨越换行符）。
    CursorLeft,
    /// 光标右移。
    CursorRight,
    /// 行首（Home / Ctrl+A；照 `keybindings.ts:247` `cursorLineStart`）。
    LineStart,
    /// 行尾（End / Ctrl+E；`keybindings.ts:248` `cursorLineEnd`）。
    LineEnd,
    /// 向后删一个字符（Delete；`keybindings.ts:254` `deleteCharForward`）。
    Delete,
    /// 删到行尾（Ctrl+K；`keybindings.ts:258` `deleteToLineEnd`）。
    DeleteToLineEnd,
    /// 删到行首（Ctrl+U；`keybindings.ts:257` `deleteToLineStart`）。
    DeleteToLineStart,
    /// 向前删一个词（Ctrl+W；`keybindings.ts:255` `deleteWordBackward`）。
    DeleteWordBackward,
    /// Tab：菜单打开时**应用当前选中项**（ref `autocomplete.ts:389-418`）。
    Tab,
    /// 插入换行而不提交（Ctrl+J；`keybindings.ts:262` `tui.input.newLine`，实测候选见 P7-5/W1）。
    Newline,
    /// 历史：上一条（Up）。
    HistoryPrev,
    /// 历史：下一条（Down）。
    HistoryNext,
    /// 清空编辑器（Ctrl+C；ref `core/keybindings.ts:94` `"app.clear": ctrl+c`）。
    Clear,
    /// 括号粘贴（`\x1b[200~…\x1b[201~`）：整段文本入编辑器、**不提交**（T12 的 TS 基线）。
    Paste(String),
    /// Esc：关闭补全弹层。
    Escape,
    /// 终止信号（SIGINT/SIGTERM）或 `/quit`。
    Quit,
    /// agent loop：助手文本的**增量**（流式）片段；连续增量合并进同一条助手消息。
    AssistantDelta(String),
    /// 助手消息尾部的状态提示（`Error: …` / 截断 / 中止）。
    ///
    /// ref `assistant-message.ts:182-199`：提示是消息容器里 `Spacer(1)` 之后的**独立** `Text`
    /// ⇒ 已有一段正文时它会另起一行（中间一个空行），正文为空时就是首行。
    AssistantNotice(String),
    /// 分支摘要（`/tree` 选 `Summarize` 后追加一块）。
    BranchSummary(String),
    /// **绝对**用量（会话累计）：直接覆盖 footer 上的 `↑n ↓n $x` 与百分比。
    ///
    /// ref `footer.ts:86-105` 是遍历**整个会话的条目**求和，不是事件累加；`-c`/`/resume`
    /// 与压缩之后都靠这条口径保持一致。
    UsageTotals {
        /// 会话累计输入 token。
        input: u64,
        /// 会话累计输出 token。
        output: u64,
        /// 会话累计费用（0 时不显示）。
        cost: f64,
        /// 上下文占用百分比（`None` = `?`）。
        percent: Option<f64>,
        /// 窗口文本（如 `128k`）。
        window: String,
    },
    /// `shift+tab`（ref `keybindings.ts:100-102` 的 `app.thinking.cycle`）。
    BackTab,
    /// 状态通知（`/settings` 改值后 ref 会 `showStatus(...)` 往会话区追加一行）。
    Status(String),
    /// 思考增量（ref `assistant-message.ts:118-150` 的 `thinking` 内容块；默认**显示**，
    /// `hideThinkingBlock` 默认 `false`，见 `core/settings-manager.ts:978-979`）。
    ThinkingDelta(String),
    /// agent loop：一次工具调用开始（golden/T06 只显示调用行）。
    ToolCall {
        /// 工具名。
        name: String,
        /// 参数摘要（如 `input.txt`）。
        summary: String,
    },
    /// agent loop：更新 footer 左段（`↑n ↓n x%/Nk (auto)`；照 `footer.ts:132-135`、`:150-155`）。
    Usage {
        /// 累计输入 token（0 时不显示 `↑`）。
        input: u64,
        /// 累计输出 token（0 时不显示 `↓`）。
        output: u64,
        /// 上下文占用百分比（`None` ⇒ `?`，照 `footer.ts:112-113`）。
        percent: Option<f64>,
        /// 上下文窗口展示串（如 `128k`）。
        window: String,
        /// 是否开启自动压缩（`footer.ts:151`）。
        auto: bool,
        /// 累计费用（美元）。`None` = 不显示（实时运行时的 usage 事件不带费用，
        /// 只有 resume/continue 从会话里累加出来时才带）。
        cost: Option<f64>,
    },
    /// agent loop：是否正在运行（golden 里对应 `Working` 行）。
    Working(bool),
}

/// 处理事件后的动作（交给终端层执行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    /// 无副作用（重绘即可）。
    Redraw,
    /// 提交输入（内容已从编辑器移除）。
    Submit(String),
    /// 已知内建命令（内容已从编辑器移除）：由 pi-cli 执行。
    Command(String),
    /// 退出（终端层负责恢复终端）。
    Exit,
}

/// 按**字符**计的输入长度（宽字符算一个）。
fn char_count(input: &str) -> usize {
    input.chars().count()
}

/// 光标所在行的起点（字符下标）。
fn line_start(input: &str, cursor: usize) -> usize {
    let chars: Vec<char> = input.chars().collect();
    let mut index = cursor.min(chars.len());
    while index > 0 && chars[index - 1] != '\n' {
        index -= 1;
    }
    index
}

/// 光标所在行的终点（字符下标，不含换行符）。
fn line_end(input: &str, cursor: usize) -> usize {
    let chars: Vec<char> = input.chars().collect();
    let mut index = cursor.min(chars.len());
    while index < chars.len() && chars[index] != '\n' {
        index += 1;
    }
    index
}

/// 在光标处插入一个字符（光标随后前进一格）；插入后不再是历史浏览态。
fn insert_at_cursor(state: &mut TuiState, ch: char) {
    let mut chars: Vec<char> = state.input.chars().collect();
    let at = state.cursor.min(chars.len());
    chars.insert(at, ch);
    state.cursor = at + 1;
    state.input = chars.into_iter().collect();
    state.history_pos = None;
}

/// 删除光标处（右侧）的一个字符。
fn delete_at_cursor(state: &mut TuiState) {
    let mut chars: Vec<char> = state.input.chars().collect();
    if state.cursor < chars.len() {
        chars.remove(state.cursor);
        state.input = chars.into_iter().collect();
    }
}

/// 往会话区写一行状态（ref `showStatus` 语义：**最后一条已是状态行时原地替换**）。
///
/// `shift+tab` 连按多次档位时，屏幕上只应留下**一行** `Thinking level: X`。
fn push_status(state: &mut TuiState, message: &str) {
    match state.transcript.last_mut() {
        Some(TranscriptItem::Status(text)) => *text = message.to_string(),
        _other => state
            .transcript
            .push(TranscriptItem::Status(message.to_string())),
    }
}

/// 处理一个事件；纯函数式（只改 `state`，不做 I/O）。
pub fn apply(state: &mut TuiState, event: TuiEvent) -> TuiAction {
    if state.exited {
        return TuiAction::Exit;
    }
    match event {
        TuiEvent::BranchSummary(message) => {
            state.transcript.push(TranscriptItem::BranchSummary {
                summary: message,
                expanded: false,
            });
            TuiAction::Redraw
        }
        TuiEvent::Status(message) => {
            // ref `showStatus`（`:3695-3703`）：最后一条已是状态行时**原地更新**。
            match state.transcript.last_mut() {
                Some(TranscriptItem::Status(text)) => *text = message,
                Some(_) | None => state.transcript.push(TranscriptItem::Status(message)),
            }
            TuiAction::Redraw
        }
        TuiEvent::ThinkingDelta(delta) => {
            match state.transcript.last_mut() {
                Some(TranscriptItem::Thinking(text)) => text.push_str(&delta),
                Some(TranscriptItem::User(_))
                | Some(TranscriptItem::Assistant(_))
                | Some(TranscriptItem::BranchSummary { .. })
                | Some(TranscriptItem::Status(_))
                | Some(TranscriptItem::ToolCall { .. })
                | None => state.transcript.push(TranscriptItem::Thinking(delta)),
            }
            TuiAction::Redraw
        }
        TuiEvent::BackTab => {
            // ref `interactive-mode.ts:4367-4378`：非推理模型给
            // `Current model does not support thinking`；否则循环并显示 `Thinking level: X`。
            if state.available_thinking_levels.len() <= 1 {
                push_status(state, "Current model does not support thinking");
            } else {
                let index = state
                    .available_thinking_levels
                    .iter()
                    .position(|level| *level == state.thinking_level)
                    .unwrap_or(0);
                let next = (index + 1) % state.available_thinking_levels.len();
                state.thinking_level = state.available_thinking_levels[next].clone();
                let message = format!("Thinking level: {}", state.thinking_level);
                push_status(state, &message);
            }
            TuiAction::Redraw
        }
        TuiEvent::AssistantNotice(text) => {
            match state.transcript.last_mut() {
                // 正文为空：直接写进这条（渲染出来就是"空行 + 提示"）。
                Some(TranscriptItem::Assistant(existing)) if existing.is_empty() => {
                    existing.push_str(&text);
                }
                // 已有正文：另起一条（渲染时自带走一条前导空行 = ref 的 `Spacer(1)`）。
                Some(TranscriptItem::Assistant(_))
                | None
                | Some(TranscriptItem::User(_))
                | Some(TranscriptItem::Thinking(_))
                | Some(TranscriptItem::Status(_))
                | Some(TranscriptItem::BranchSummary { .. })
                | Some(TranscriptItem::ToolCall { .. }) => {
                    state.transcript.push(TranscriptItem::Assistant(text));
                }
            }
            TuiAction::Redraw
        }
        TuiEvent::AssistantDelta(delta) => {
            // 流式：合并进**最后一条**助手消息；上一条不是助手则新建。
            match state.transcript.last_mut() {
                Some(TranscriptItem::Assistant(text)) => text.push_str(&delta),
                Some(TranscriptItem::User(_))
                | Some(TranscriptItem::BranchSummary { .. })
                | Some(TranscriptItem::Status(_))
                | Some(TranscriptItem::Thinking(_))
                | Some(TranscriptItem::ToolCall { .. })
                | None => state.transcript.push(TranscriptItem::Assistant(delta)),
            }
            TuiAction::Redraw
        }
        TuiEvent::ToolCall { name, summary } => {
            state
                .transcript
                .push(TranscriptItem::ToolCall { name, summary });
            TuiAction::Redraw
        }
        TuiEvent::Usage {
            input,
            output,
            percent,
            window,
            auto,
            cost,
        } => {
            // footer 的用量口径（实测对齐，见 T45/T70）：
            // - 有 usage 的一轮 ⇒ **替换**为这一轮的值（ref 的 `usageTotals` 在该场景等于本轮值）；
            // - 没有 usage 的一轮（失败/中止）⇒ **保留**已有值，不能被清零。
            if input > 0 || output > 0 {
                state.usage_input = input;
                state.usage_output = output;
            }
            if let Some(total) = cost {
                state.usage_cost = total;
            }
            let parts_for_cost: Option<String> = if state.usage_cost == 0.0 {
                None
            } else {
                Some(format!("${:.3}", state.usage_cost))
            };
            let mut parts: Vec<String> = Vec::new();
            if state.usage_input > 0 {
                parts.push(format!("↑{}", state.usage_input));
            }
            if state.usage_output > 0 {
                parts.push(format!("↓{}", state.usage_output));
            }
            // 照 `footer.ts:132-135/150-155`：`↑n ↓n x%/Nk (auto)`；无用量时只有百分比段。
            let percent_text = match percent {
                Some(value) => format!("{value:.1}"),
                None => "?".to_string(),
            };
            let auto_text = if auto { " (auto)" } else { "" };
            if let Some(cost_text) = parts_for_cost {
                parts.push(cost_text);
            }
            parts.push(format!("{percent_text}%/{window}{auto_text}"));
            state.status = parts.join(" ");
            TuiAction::Redraw
        }
        TuiEvent::UsageTotals {
            input,
            output,
            cost,
            percent,
            window,
        } => {
            state.usage_input = input;
            state.usage_output = output;
            state.usage_cost = cost;
            let percent_text = match percent {
                Some(value) => format!("{value:.1}"),
                None => "?".to_string(),
            };
            let mut parts: Vec<String> = Vec::new();
            if state.usage_input > 0 {
                parts.push(format!("↑{}", state.usage_input));
            }
            if state.usage_output > 0 {
                parts.push(format!("↓{}", state.usage_output));
            }
            if state.usage_cost != 0.0 {
                parts.push(format!("${:.3}", state.usage_cost));
            }
            parts.push(format!("{percent_text}%/{window} (auto)"));
            state.status = parts.join(" ");
            TuiAction::Redraw
        }
        TuiEvent::Working(on) => {
            state.working = on;
            TuiAction::Redraw
        }
        TuiEvent::CtrlO
        | TuiEvent::CtrlT
        | TuiEvent::CtrlL
        | TuiEvent::CtrlP
        | TuiEvent::CtrlShiftO
        | TuiEvent::CtrlS
        | TuiEvent::CtrlN
        | TuiEvent::CtrlR
        | TuiEvent::CtrlX
        | TuiEvent::CtrlLeft
        | TuiEvent::CtrlRight => {
            // 主界面里这些键无绑定（`ctrl+o` 的"显示更多"未实现）⇒ 不改变内容。
            TuiAction::Redraw
        }
        TuiEvent::Char(ch) => {
            insert_at_cursor(state, ch);
            // `/` 触发补全弹层（T14 实测；输入任何其它内容后自动关闭）。
            // P7-6am：补全在**输入以 `/` 开头且未出现空格**时保持打开（原来只在恰好等于 "/" 时打开，
            // 所以输入 `/s` 后菜单立刻消失 —— 用户真机反馈的"没有自动补全"）。
            state.completion_open = should_open_completion(&state.input);
            state.completion_selected = 0;
            TuiAction::Redraw
        }
        TuiEvent::CursorLeft => {
            // 占位符是**原子段**（ref `editor.ts:38-72` 的 `segmentWithMarkers`）：
            // 光标从占位符尾部往左一次跳到它的开头。
            if let Some((start, _end)) = marker_before_cursor(state) {
                state.cursor = start;
            } else {
                state.cursor = state.cursor.saturating_sub(1);
            }
            TuiAction::Redraw
        }
        TuiEvent::CursorRight => {
            // 同理：从占位符开头往右一次跳到它的末尾。
            if let Some((_start, end)) = marker_spans(state)
                .into_iter()
                .find(|(start, _end)| *start == state.cursor)
            {
                state.cursor = end;
            } else if state.cursor < char_count(&state.input) {
                state.cursor += 1;
            }
            TuiAction::Redraw
        }
        TuiEvent::LineStart => {
            state.cursor = line_start(&state.input, state.cursor);
            TuiAction::Redraw
        }
        TuiEvent::LineEnd => {
            state.cursor = line_end(&state.input, state.cursor);
            TuiAction::Redraw
        }
        TuiEvent::Delete => {
            // 前向删除同样整段删占位符（ref 的原子段）。
            if let Some((start, end)) = marker_spans(state)
                .into_iter()
                .find(|(start, _end)| *start == state.cursor)
            {
                if let Some(id) = marker_id_at(&state.input, start) {
                    drop_paste_marker(state, id, start, end);
                } else {
                    remove_range(state, start, end);
                }
                refresh_completion(state);
                return TuiAction::Redraw;
            }
            delete_at_cursor(state);
            // ref `editor.ts:1809`：前向删除后同样重算补全。
            refresh_completion(state);
            TuiAction::Redraw
        }
        TuiEvent::DeleteToLineEnd => {
            let end = line_end(&state.input, state.cursor);
            let mut chars: Vec<char> = state.input.chars().collect();
            chars.drain(state.cursor..end);
            state.input = chars.into_iter().collect();
            TuiAction::Redraw
        }
        TuiEvent::DeleteToLineStart => {
            let start = line_start(&state.input, state.cursor);
            let mut chars: Vec<char> = state.input.chars().collect();
            chars.drain(start..state.cursor);
            state.cursor = start;
            state.input = chars.into_iter().collect();
            TuiAction::Redraw
        }
        TuiEvent::DeleteWordBackward => {
            // ref `components/editor.ts:1689-1727` `deleteWordBackwards`：
            // ① 光标在**行首**（上一字符是 `\n`）⇒ 只删那个换行（与上一行合并），**不跨行删词**；
            // ② 否则跳过尾部空白再删一个词，且**不跨越 `\n`**。
            let mut chars: Vec<char> = state.input.chars().collect();
            let mut start = state.cursor;
            if start > 0 && chars[start - 1] == '\n' {
                chars.drain(start - 1..start);
                state.cursor = start - 1;
                state.input = chars.into_iter().collect();
                return TuiAction::Redraw;
            }
            while start > 0 && chars[start - 1].is_whitespace() && chars[start - 1] != '\n' {
                start -= 1;
            }
            while start > 0 && !chars[start - 1].is_whitespace() {
                start -= 1;
            }
            chars.drain(start..state.cursor);
            state.cursor = start;
            state.input = chars.into_iter().collect();
            TuiAction::Redraw
        }
        TuiEvent::Tab => {
            // ref：菜单打开且有选中项 ⇒ 用 `/<item.value> ` 替换已输入的斜杠前缀，光标落在空格之后
            // （`autocomplete.ts:404-418`：`newLine = beforePrefix + "/" + value + " " + afterCursor`，
            //  `cursorCol = beforePrefix.length + value.length + 2`），随后关闭菜单。
            if state.completion_open && state.input.starts_with('/') && !state.input.contains(' ') {
                let typed = state.input.trim_start_matches('/');
                // 选中的是菜单里高亮那一项（↑/↓ 可移动）；菜单里没有对应项时退回"前缀匹配/首项"。
                let matches = completion_matches(&state.input);
                let selected = matches
                    .get(
                        state
                            .completion_selected
                            .min(matches.len().saturating_sub(1)),
                    )
                    .map(|(name, _)| (*name, ()))
                    .or_else(|| {
                        matches
                            .iter()
                            .find(|(name, _)| name.starts_with(typed))
                            .map(|(name, _)| (*name, ()))
                    })
                    .or_else(|| matches.first().map(|(name, _)| (*name, ())));
                if let Some((name, ())) = selected {
                    state.input = format!("/{name} ");
                    state.cursor = state.input.chars().count();
                    state.completion_open = false;
                }
            }
            TuiAction::Redraw
        }
        TuiEvent::Newline => {
            insert_at_cursor(state, '\n');
            TuiAction::Redraw
        }
        TuiEvent::HistoryPrev => {
            // 补全菜单打开时，↑/↓ 移动**菜单选中项**（不翻历史）——ref `editor.ts:763-766`。
            if state.completion_open {
                let total = completion_matches(&state.input).len();
                if total > 0 {
                    state.completion_selected = if state.completion_selected == 0 {
                        total - 1
                    } else {
                        state.completion_selected - 1
                    };
                }
                return TuiAction::Redraw;
            }
            // 面板打开时，Up/Down 优先移动面板选中项（选择器语义），不切历史。
            if state.panel.is_some() {
                state.panel_selection = state.panel_selection.saturating_sub(1);
                return TuiAction::Redraw;
            }
            if state.history.is_empty() {
                return TuiAction::Redraw;
            }
            let next = match state.history_pos {
                None => state.history.len() - 1,
                Some(0) => 0,
                Some(index) => index - 1,
            };
            state.history_pos = Some(next);
            state.input = state.history[next].clone();
            state.cursor = char_count(&state.input);
            TuiAction::Redraw
        }
        TuiEvent::HistoryNext => {
            // 同 ↑：菜单打开时移动菜单选中项（ref `editor.ts:763-766`）。
            if state.completion_open {
                let total = completion_matches(&state.input).len();
                if total > 0 {
                    state.completion_selected = (state.completion_selected + 1) % total;
                }
                return TuiAction::Redraw;
            }
            if state.panel.is_some() {
                state.panel_selection = state.panel_selection.saturating_add(1);
                return TuiAction::Redraw;
            }
            let Some(index) = state.history_pos else {
                return TuiAction::Redraw;
            };
            if index + 1 >= state.history.len() {
                state.history_pos = None;
                state.input.clear();
            } else {
                state.history_pos = Some(index + 1);
                state.input = state.history[index + 1].clone();
            }
            state.cursor = char_count(&state.input);
            TuiAction::Redraw
        }
        TuiEvent::Clear => {
            state.input.clear();
            state.cursor = 0;
            state.completion_open = false;
            state.history_pos = None;
            TuiAction::Redraw
        }
        TuiEvent::Paste(text) => {
            // ref `editor.ts:1276-1320`：先滤掉控制字符（保留 `\n`），
            // 再按"超过 10 行 / 超过 1000 字符"折叠成 `[paste #N +L lines]` / `[paste #N C chars]`。
            // 先做 CRLF/CR → LF 归一（保留本实现原有的括号粘贴行为），
            // 再按 ref `editor.ts:1280-1284` 滤掉其余控制字符（保留 `\n`）。
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            let filtered: String = normalized
                .chars()
                .filter(|ch| *ch == '\n' || (*ch as u32) >= 32)
                .collect();
            let lines: Vec<&str> = filtered.split('\n').collect();
            if lines.len() > 10 || filtered.chars().count() > 1000 {
                state.paste_counter += 1;
                let id = state.paste_counter;
                let marker = if lines.len() > 10 {
                    format!("[paste #{id} +{} lines]", lines.len())
                } else {
                    format!("[paste #{id} {} chars]", filtered.chars().count())
                };
                state.pastes.insert(id, filtered);
                for ch in marker.chars() {
                    insert_at_cursor(state, ch);
                }
                return TuiAction::Redraw;
            }
            // 小段粘贴：同样只用过滤后的文本（含 CRLF 只需归一 `\r`）。
            // 粘贴文本按字符插入；**CRLF/CR 都归一成单个 LF**（`\r\n` 不能变成两个换行）。
            let mut previous_was_cr = false;
            for ch in filtered.chars() {
                match ch {
                    '\r' => {
                        insert_at_cursor(state, '\n');
                        previous_was_cr = true;
                    }
                    '\n' => {
                        if !previous_was_cr {
                            insert_at_cursor(state, '\n');
                        }
                        previous_was_cr = false;
                    }
                    other => {
                        insert_at_cursor(state, other);
                        previous_was_cr = false;
                    }
                }
            }
            TuiAction::Redraw
        }
        TuiEvent::Escape => {
            state.completion_open = false;
            state.close_panel();
            TuiAction::Redraw
        }
        TuiEvent::Backspace => {
            // 占位符是**原子段**（ref `segmentWithMarkers`）：退格一次删掉整个 `[paste #N …]`。
            if let Some((start, end)) = marker_before_cursor(state) {
                if let Some(id) = marker_id_at(&state.input, start) {
                    drop_paste_marker(state, id, start, end);
                } else {
                    remove_range(state, start, end);
                }
                refresh_completion(state);
                return TuiAction::Redraw;
            }
            if state.cursor > 0 {
                let mut chars: Vec<char> = state.input.chars().collect();
                chars.remove(state.cursor - 1);
                state.cursor -= 1;
                state.input = chars.into_iter().collect();
            }
            // ref `editor.ts:1444`：退格后要**重新触发/更新**补全（真机反馈：`/tree` 删字符后菜单消失）。
            refresh_completion(state);
            TuiAction::Redraw
        }
        TuiEvent::Enter => {
            // ref `editor.ts:788-800`：补全菜单打开时，Enter 与 Tab 一样是"接受选中项"，
            // **不提交**（提交要等菜单关闭后再按一次）。
            if state.completion_open && state.input.starts_with('/') && !state.input.contains(' ') {
                let matches = completion_matches(&state.input);
                if !matches.is_empty() {
                    let index = state.completion_selected.min(matches.len() - 1);
                    let (name, _) = matches[index];
                    // 接受补全**等于当前输入**（`/tree` → `/tree `）时没有可"接受"的内容，
                    // 这时 Enter 走提交（`T04`/`T18` 的 golden 就是这一步打开面板）。
                    if format!("/{name}") != state.input.trim_end() {
                        state.input = format!("/{name} ");
                        state.cursor = state.input.chars().count();
                        state.completion_open = false;
                        state.completion_selected = 0;
                        return TuiAction::Redraw;
                    }
                    state.completion_open = false;
                    state.completion_selected = 0;
                }
            }
            // ref `editor.ts:1084-1096` 的 `expandPasteMarkers`：提交前把
            // `[paste #N +L lines]` / `[paste #N C chars]` 还原成原文。
            let text = expand_paste_markers(&state.pastes, &state.input);
            state.input.clear();
            let trimmed = text.trim().to_string();
            // 历史：提交过的非空输入按顺序记下（T10 的 Up/Down 在上面移动）。
            if !trimmed.is_empty() {
                state.history.push(trimmed.clone());
            }
            state.history_pos = None;
            state.cursor = 0;
            state.completion_open = false;
            state.pastes.clear();
            state.paste_counter = 0;
            if trimmed == "/quit" {
                // ref `interactive-mode.ts:3218-3221`：`if (text === "/quit") { this.editor.setText(""); await this.shutdown(); }`
                state.exited = true;
                return TuiAction::Exit;
            }
            if trimmed.is_empty() {
                return TuiAction::Redraw;
            }
            // 已知内建命令 ⇒ 交给 pi-cli 执行（`TuiAction::Command`）；否则按普通输入提交。
            // **未知的 `/xxx` 也是普通输入**（T18 实测：TS 把它当 prompt 发给模型）。
            if let Some(name) = trimmed.strip_prefix('/') {
                let name = name.split_whitespace().next().unwrap_or_default();
                if builtin_command(name) {
                    return TuiAction::Command(name.to_string());
                }
            }
            TuiAction::Submit(trimmed)
        }
        TuiEvent::CtrlD => {
            if state.input.is_empty() {
                state.exited = true;
                TuiAction::Exit
            } else {
                // ref：`tui.editor.deleteCharForward` = ["delete","ctrl+d"]（`tui/src/keybindings.ts:120-122`）
                // ⇒ 非空时 **删光标处那一个字符**（向后删），不是删末尾；`app.exit` 只描述"空编辑器时退出"
                // （`coding-agent/src/core/keybindings.ts:95`）。
                delete_at_cursor(state);
                TuiAction::Redraw
            }
        }
        TuiEvent::Quit => {
            state.exited = true;
            TuiAction::Exit
        }
    }
}

/// 把字符串按显示宽度裁剪到 `width` 列（不补空格）。
#[must_use]
pub fn clamp_to_width(text: &str, width: usize) -> String {
    // P7-6al/F：**热路径**——旧实现每个字符都 `ch.to_string()` 再算宽度（每帧约 27 万次分配，
    // N=3000 时 render 占 35 ms）。`UnicodeWidthChar::width` 与 `UnicodeWidthStr::width(单字符)`
    // 语义等价（同一张宽度表），**输出字节完全不变**。
    // 快路径：整行本来就装得下 ⇒ 直接返回（省掉逐字符循环；宽度算法仍是同一张表）
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut used = 0usize;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// 渲染缓存（P7-6am/P'）：按「条目内容 + 宽度」缓存**已完成条目**渲染出的行。
///
/// 失效条件（任一即整份丢弃）：`cols` 变化、条目数变少、某条目内容与缓存键不同。
/// 只缓存 transcript 条目的行；头部、面板、状态区、spinner 每帧重算（它们本来就便宜，且随状态变化）。
#[derive(Debug, Default, Clone)]
pub struct RenderCache {
    width: usize,
    entries: Vec<(String, Vec<String>)>,
}

impl RenderCache {
    /// 新建空缓存。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// 按 auto-compact 开关重写状态行的 ` (auto)` 后缀（ref `footer.ts:151`）。
///
/// 状态行形态固定为 `<pct>%/<window>[ (auto)]`（见 [`TuiEvent::Usage`]），
/// 因此这里只做后缀增删，不动其余部分。
pub fn set_auto_compact(state: &mut TuiState, enabled: bool) {
    let base = state
        .status
        .strip_suffix(" (auto)")
        .unwrap_or(state.status.as_str())
        .to_string();
    state.status = if enabled {
        format!("{base} (auto)")
    } else {
        base
    };
}

/// 字符串的**显示宽度**（CJK 双宽；与 `clamp_to_width`/`wrap_to_width` 同一口径）。
#[must_use]
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// 把一行**折行**到给定显示宽度（P7-6ao/E：长段落不能截断，必须折行）。
///
/// 依据：ref 的 `Markdown` 组件 `render(width)` 会把超宽段落折行（`components/markdown.ts`），
/// 我们原来用 `clamp_to_width` 直接**截断**，导致长回复只显示第一行的前 100 列。
/// 折行策略：按显示宽度贪心累积，优先在**空格**处断开；无空格时按字符硬断。
#[must_use]
pub fn wrap_to_width(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    if UnicodeWidthStr::width(text) <= width {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;
    let mut last_space: Option<(usize, usize)> = None; // (字节偏移, 宽度)
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            // 恰好填满后遇到**空格**时，这个空格就是换行点：直接断行并丢掉它，
            // 不能回退到上一个空格（否则会白白少放一个词，见 T30 的描述折行差异）。
            if ch == ' ' {
                out.push(std::mem::take(&mut current));
                used = 0;
                last_space = None;
                continue;
            }
            match last_space {
                Some((byte, _space_width)) if !current.is_empty() => {
                    let rest = current[byte..].to_string();
                    current.truncate(byte);
                    out.push(current.trim_end().to_string());
                    current = rest.trim_start().to_string();
                    used = UnicodeWidthStr::width(current.as_str());
                    if used + w <= width {
                        last_space = None;
                        current.push(ch);
                        used += w;
                        continue;
                    }
                    out.push(std::mem::take(&mut current));
                    used = 0;
                    last_space = None;
                    current.push(ch);
                    used += w;
                }
                other => {
                    let _ = other;
                    if !current.is_empty() {
                        out.push(std::mem::take(&mut current));
                    }
                    used = 0;
                    last_space = None;
                    current.push(ch);
                    used += w;
                }
            }
            continue;
        }
        if ch == ' ' {
            last_space = Some((current.len(), used));
        }
        current.push(ch);
        used += w;
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// 渲染成 `rows` 行、每行 `cols` 列（纯函数，便于单测与快照比对）。
///
/// 布局（自上而下）：头部（版本/提示/欢迎）→ 空行 → `[Context]` 段 → 边框与编辑器 →
/// 状态行（cwd + 右侧 `status`）。未用到的行留空。
#[must_use]
pub fn render_all(state: &TuiState, cols: usize, rows: usize) -> Vec<String> {
    render_all_inner(state, cols, rows)
}

/// 与 [`render_all`] 相同；`r3_extra = true` 时在**工具条目前多补 1 空行**（**仅 r3**）：
/// ref `tool-execution.ts:105` 的 `Box(1, 1, …)` 顶部 padding（见 `notes/reports/P7-6p-predict-w4d.md`）。
/// `false` 时与 `render_all` 既有输出**逐字节一致**（R1/R2 走这条）。
fn render_all_inner(state: &TuiState, cols: usize, rows: usize) -> Vec<String> {
    render_all_cached_inner(state, cols, rows, None)
}

/// 带缓存的内核；`cache = None` 时行为与旧实现**逐字节一致**。
fn render_all_cached_inner(
    state: &TuiState,
    cols: usize,
    rows: usize,
    mut cache: Option<&mut RenderCache>,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::with_capacity(rows);
    lines.push(String::new());
    // ref `:958`：`pi v<version>`（头部件内边距 → 一个前导空格）
    // golden-tui 的启动屏显示的是**上游 pi 的版本**（0.87.1，见 tag/`ref/`），不是本 crate 的包版本。
    let _ = &state.version;
    lines.push(clamp_to_width(&format!(" pi v{}", DISPLAY_VERSION), cols));
    // ref `:985-990`：键位提示行（默认绑定见 `core/keybindings.ts:93-95` 等）
    lines.push(clamp_to_width(
        " escape interrupt · ctrl+c/ctrl+d clear/exit · / commands · ! bash · ctrl+o more",
        cols,
    ));
    // ref `:991-994`
    lines.push(clamp_to_width(
        " Press ctrl+o to show full startup help and loaded resources.",
        cols,
    ));
    lines.push(String::new());
    // ref `:995-998`
    lines.push(clamp_to_width(
        " Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.",
        cols,
    ));
    lines.push(String::new());
    // ref `:1770-1784`：`[Context]` 段（只显示路径）
    if !state.context_files.is_empty() {
        // P7-6o(续4)/W0：`[Context]` 段前的段落空行。ref `interactive-mode.ts:1776`
        // `this.loadedResourcesContainer.addChild(new Spacer(1));`（在 `addLoadedSection("Context", …)` `:1784` 之前）。
        lines.push(String::new());
        lines.push("[Context]".to_string());
        for path in &state.context_files {
            // golden 只显示 basename（P7-3 W3 实测：golden 行为 `  AGENTS.md`）。
            let name = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            lines.push(clamp_to_width(&format!("  {name}"), cols));
        }
        lines.push(String::new());
        if state.transcript.is_empty() && state.panel.is_none() {
            // r3 专属：`[Context]` 段末到编辑器上边框之间在 golden 中有 **2** 个空行
            // （T02/T16 实测：`  AGENTS.md` / 空 / 空 / 边框），pi-rs 目前只 1 个。
            lines.push(String::new());
        }
    }
    // 会话区（W3-2）：用户消息、助手 markdown、工具调用块，按顺序输出；每项后留一空行。
    // P7-6am/P'：条目行按「内容 + 宽度」缓存；缓存未命中或失效时才重新渲染。
    if let Some(cache) = cache.as_deref_mut() {
        if cache.width != cols {
            cache.entries.clear();
            cache.width = cols;
        }
        if cache.entries.len() > state.transcript.len() {
            cache.entries.truncate(state.transcript.len());
        }
    }
    for (item_index, item) in state.transcript.iter().enumerate() {
        let cache_key = match item {
            TranscriptItem::User(text) => format!("u\u{1}{text}"),
            TranscriptItem::Assistant(text) => format!("a\u{1}{text}"),
            TranscriptItem::BranchSummary { summary, expanded } => {
                format!("b\u{1}{expanded}\u{1}{summary}")
            }
            TranscriptItem::Status(text) => format!("s\u{1}{text}"),
            TranscriptItem::Thinking(text) => format!("k\u{1}{text}"),
            TranscriptItem::ToolCall { name, summary } => format!("t\u{1}{name}\u{1}{summary}"),
        };
        if let Some(cache) = cache.as_deref_mut() {
            if let Some((key, rendered)) = cache.entries.get(item_index) {
                if key == &cache_key {
                    lines.push(String::new());
                    lines.extend(rendered.iter().cloned());
                    continue;
                }
            }
        }
        let before = lines.len();
        // P7-6o(续4)/W0：每条聊天条目前的段落空行（`Hint` 例外：ref 直接 addChild 紧贴前一条）。ref `chatContainer` 级
        // `addChild(new Spacer(1))`（如 `interactive-mode.ts:3678/3768/3775/3787/3800/3964/3979/4021`）。
        lines.push(String::new());
        match item {
            TranscriptItem::User(text) => {
                // 多行用户消息（T09 after-submit 的 golden 实测）：**每行都带一个前导空格**。
                for line in text.split('\n') {
                    lines.push(clamp_to_width(&format!(" {line}"), cols));
                }
                lines.push(String::new());
            }
            TranscriptItem::BranchSummary { summary, expanded } => {
                // ref `branch-summary-message.ts:32-53`：`[branch]` 标签 + 空行 + 折叠/展开内容。
                lines.push(clamp_to_width(" [branch]", cols));
                lines.push(String::new());
                if *expanded {
                    lines.push(clamp_to_width(" Branch Summary", cols));
                    lines.push(String::new());
                    for line in render_markdown(summary) {
                        for wrapped in wrap_to_width(&line, cols) {
                            lines.push(wrapped);
                        }
                    }
                } else {
                    lines.push(clamp_to_width(" Branch summary (ctrl+o to expand)", cols));
                }
                lines.push(String::new());
            }
            TranscriptItem::Status(text) => {
                // ref `showStatus`（`interactive-mode.ts:3709-3712`）加的是 `Spacer(1)` + `Text(msg,1,0)`：
                // 会话区里表现为"状态行 + 一个空行"（T32 golden 实测），因此这里输出两行。
                lines.push(clamp_to_width(&format!(" {text}"), cols));
                lines.push(String::new());
            }
            TranscriptItem::Thinking(text) => {
                for line in render_markdown(text) {
                    for wrapped in wrap_to_width(&line, cols) {
                        lines.push(wrapped);
                    }
                }
                lines.push(String::new());
            }
            TranscriptItem::Assistant(text) => {
                for line in render_markdown(text) {
                    // 长段落必须**折行**而不是截断（否则长回复只显示首行，见 P7-6ao/E）。
                    for wrapped in wrap_to_width(&line, cols) {
                        lines.push(wrapped);
                    }
                }
                lines.push(String::new());
            }
            TranscriptItem::ToolCall { name, summary } => {
                let text = if summary.is_empty() {
                    format!(" {name}")
                } else {
                    format!(" {name} {summary}")
                };
                // r3 专属：工具条目前补 ref 的 `Box` 顶 padding（`tool-execution.ts:105`）。
                lines.push(String::new());
                lines.push(clamp_to_width(&text, cols));
                // golden/T06 实测：工具调用块之后也有一个空行（行 12 调用、13 空、14 助手文本）。
                lines.push(String::new());
            }
        }
        if let Some(cache) = cache.as_deref_mut() {
            let rendered: Vec<String> = lines[before + 1..].to_vec();
            let slot = (cache_key, rendered);
            if cache.entries.len() > item_index {
                cache.entries[item_index] = slot;
            } else {
                cache.entries.push(slot);
            }
        }
    }
    // 运行中：画一行 `Working`（golden 实测形状 `── <SPINNER> Working ──…`；spinner 用 braille 字符，
    // 归一化会把它掩码为 `<SPINNER>`）。作用：让**物理终端多滚一行** —— 会话结束时本行撤掉，但终端
    // 已经滚上去的那一行不会回来，这正是 T05 的 TS 屏幕比 pi-rs 少一个前导空行的原因。
    // P7-6p(续6)/W1：r3 把 `Working` **嵌入编辑器内容行**（ref `embedWorkingStatus: true`，
    // `interactive-mode.ts:603-606` + `showStatusIndicator` 嵌入即不加容器 `:2215-2226`），行数不变；
    // R1/R2 仍沿用"独立整行"的旧形态（`r3_extra == false`）。
    let mut working_line: Option<String> = None;
    if state.working {
        let frame = SPINNER_FRAMES
            .get(state.spinner_frame)
            .copied()
            .unwrap_or("⠋");
        let prefix = format!("── {frame} Working ");
        let used = prefix.chars().count();
        let filler = "─".repeat(cols.saturating_sub(used));
        let text = clamp_to_width(&format!("{prefix}{filler}"), cols);
        working_line = Some(text);
    }
    // 会话区挂载的面板（`/session`、`/hotkeys`：golden 实测画在编辑器上边框**之上**）。
    // P7-6r2/W3：**仅 r3** 路径把 `/hotkeys` 面板换成按 `cols` 动态生成的行（R1/R2 保持旧常量）。
    // 识别方式：面板首行 trim 后等于 `Keyboard Shortcuts`（`HOTKEYS_LINES[0]`）。
    let dynamic_hotkeys: Option<Vec<String>> = {
        state
            .panel
            .as_ref()
            .filter(|(_, lines)| {
                lines
                    .first()
                    .is_some_and(|l| l.trim() == "Keyboard Shortcuts")
            })
            .map(|_| hotkeys_panel_for_width(cols))
    };
    if let Some((PanelPlacement::Transcript, panel_lines)) = &state.panel {
        let panel_lines: &Vec<String> = dynamic_hotkeys.as_ref().unwrap_or(panel_lines);
        // P7-6m/W1：Transcript 面板前后各**两个空行**。
        // 依据 golden-tui/T04 末帧：`[Context]` → `  AGENTS.md` → 空 → 空 → 面板 → 空 → 空 → 编辑器上边框；
        // 面板前的第一个空行来自上文 `[Context]` 段末的空行，这里补第二个；面板后由 1 个改为 2 个。
        lines.push(String::new());
        if panel_lines
            .first()
            .is_some_and(|l| l.trim() == "Keyboard Shortcuts")
        {
            lines.push("─".repeat(cols)); // r3 专属：/hotkeys 面板前整宽边框（golden T15:12）
        }
        for line in panel_lines {
            lines.push(clamp_to_width(line, cols));
        }
        lines.push(String::new());
        if panel_lines
            .first()
            .is_some_and(|l| l.trim() == "Keyboard Shortcuts")
        {
            lines.push("─".repeat(cols)); // r3 专属：/hotkeys 面板后整宽边框（golden T15:102）
        }
        lines.push(String::new());
    }
    // 编辑器框：上边框（整宽）→ 内容行 → 下边框（整宽）。ref `modes/interactive/components/custom-editor.ts`
    // 与 `dynamic-border.ts`；golden 的原始流里上下边框都是 `─`×100（整宽，无前导空格）。
    let border = "─".repeat(cols);
    // P7-6p(续7)/W1：r3 专属——`/settings` 选择器（面板首行 `>`）在 golden 中
    // **编辑器上边框之前**还有 1 个空行（T17 帧0：`空 / 空 / 边框 / >`）；`/model`（首行为空串）无此空行。
    // ref 的选择器组件自带**前导 `Spacer(1)`**（`tree-selector.ts:1383` 的
    // `this.addChild(new Spacer(1))` 在 `new DynamicBorder()` 之前），它画在上边框**之前**，
    // 与 transcript 末尾的空行叠成两行（T44 家族：TS 缓冲 33 行、我们 32 行，整屏少上滚 1 行）。
    // `/settings` 的 `SettingsList` 首行是搜索框 `>`，同样自带该空行；
    // `/model` 面板的首行是 `Only showing …`，其空行被编辑器框布局吸收（T16/T26 严格 same 即证）。
    if state.panel.as_ref().is_some_and(|(p, l)| {
        matches!(p, PanelPlacement::BelowEditor)
            && l.first()
                .is_some_and(|x| matches!(x.trim(), ">" | "Session Tree"))
    }) {
        lines.push(String::new());
    }
    lines.push(border.clone());
    // 多行编辑器（T09/T12 实测：`\n` 即换行，每行带一个前导空格，编辑器框随之增高）。
    let input_lines: Vec<&str> = state.input.split('\n').collect();
    // **多行时不加前导空格**（T09/T12 的 golden 实测：`line one`/`line two` 顶格；
    // 单行时才是 ` say hi` 带一个前导空格的形态）。
    // **编辑器行一律不加前导空格**（T09/T11/T12/T13/T14 的 golden 全部如此：`hello world`、`中文🙂测试`、
    // `line one`、`/`）。之前看到的 ` say hi` 带空格是**transcript 里的用户消息**（由 `TranscriptItem::User`
    // 那一支渲染），不是编辑器内容——两者不要混用同一条规则。
    if let Some(text) = &working_line {
        // r3：`Working` 占用编辑器内容行（ref 的"嵌入"形态）⇒ 不额外增加行。
        lines.push(text.clone());
    } else if !matches!(state.panel, Some((PanelPlacement::BelowEditor, _))) {
        for line in &input_lines {
            lines.push(clamp_to_width(line, cols));
        }
    }
    // 编辑器**内部**的面板（`/settings`、`/model` 选择器）：golden 实测它们画在
    // **输入行之下、编辑器的下边框之上**；且选择器**替换**编辑器（`interactive-mode.ts:4719-4732`
    // `editorContainer.clear(); addChild(component)`）⇒ 打开面板时**不画编辑器输入行**
    // （`SettingsList` 自带搜索框 `>`，`packages/tui/src/components/settings-list.ts:108-111`；
    //  `/model` 的搜索框在 `model-selector.ts:116`，其前 `:101` 的 `Spacer(1)` 已并入常量首行）。
    if let Some((PanelPlacement::BelowEditor, panel_lines)) = &state.panel {
        for line in panel_lines {
            lines.push(clamp_to_width(line, cols));
        }
    }
    lines.push(border);
    // `/` 的命令补全弹层（T14 实测）：渲染在编辑器下边框**之下**、状态区**之上**，
    // 首项带 `→ ` 标记，名称列宽 15，末行为 `(1/N)` 计数。条目取自 TS 的 built-in 命令表
    // （该表在 pi-tui 内联 bundle 里，非 ref 源码）⇒ 这里按 golden 逐字对齐前 5 项与计数。
    if state.completion_open {
        let matches = crate::completion_matches(&state.input);
        let selected = state
            .completion_selected
            .min(matches.len().saturating_sub(1));
        for (index, (name, description)) in matches.iter().take(5).enumerate() {
            let marker = if index == selected { "→ " } else { "  " };
            lines.push(clamp_to_width(
                &format!("{marker}{name:<15}{description}"),
                cols,
            ));
        }
        // 计数：ref 的 golden 在输入 "/" 时显示 `(1/25)`（24 个内置命令 + 1 个来源未定位），
        // 过滤后按**匹配数**显示，未定位的那一项只在无过滤时计入。
        let total = matches.len() + usize::from(state.input == "/");
        let position = matches
            .len()
            .checked_sub(1)
            .map_or(1, |last| selected.min(last) + 1);
        lines.push(clamp_to_width(&format!("  ({position}/{total})"), cols));
    }
    // 状态区是**两行**（golden 实测）：第一行 cwd；第二行左边 `status`、右边模型名。
    lines.push(clamp_to_width(&state.cwd, cols));
    let status_right = clamp_to_width(&state.model, cols);
    let status_left = clamp_to_width(&state.status, cols);
    let pad = cols.saturating_sub(
        UnicodeWidthStr::width(status_left.as_str())
            + UnicodeWidthStr::width(status_right.as_str()),
    );
    lines.push(format!("{status_left}{}{status_right}", " ".repeat(pad)));
    // 聊天区视口（P7-4h/W1）：TS 的聊天区在内容超出容量时**整屏上滚**（丢最上面一行）。实测证据：
    // T05 的 TS 末帧有效行数 27、pi-rs 28，错位一行比较差异=0 ⇒ 只差一个前导空行，原因是 TS 溢出一行后
    // 上滚过（首行直接是 ` pi v0.87.1`）；T01/T06 不溢出 ⇒ 両侧一致。容量取 `rows − 7`
    // （编辑器框 3 行 + 状态区 2 行 + 2 行余量），溢出时丢最上面的 (chat_len − capacity) 行。
    // 容量实测（P7-5/W2）：`rows − 7`。依据：T10 的聊天区 23 行**不**上滚（golden 首行是空行），
    // 而 T05 的 23 行 + 运行中那一行 `Working` = 24 > 23 恰好上滚一次 ⇒ 两者同时成立。
    // **容量仍取 **：改成  会让 T05/T09 由 same 变 different（实测，违反"原先 same 不得变"）
    // ⇒ 已回退。T10 的"多一行"因此仍未解决（需要更多尺寸/场景数据才能定出真实规则）。
    lines
}

/// 在 [`render_all`] 的基础上做**视口裁剪**（R1 的现有行为：`capacity = rows − 8`）并按 `rows` 补齐。
///
/// R2（`PI_TUI_RENDERER=r2`）走的是 `render_all` + 行级 diff，**不做这里的裁剪**。
pub fn render(state: &TuiState, cols: usize, rows: usize) -> Vec<String> {
    const CHROME_LINES: usize = 5; // 编辑器框（上/内容/下）+ 状态区（cwd/用量）
    let mut lines = render_all(state, cols, rows);
    let capacity = rows.saturating_sub(CHROME_LINES + 3);
    let chat_len = lines.len().saturating_sub(CHROME_LINES);
    if chat_len > capacity {
        lines.drain(0..chat_len - capacity);
    }
    lines.truncate(rows);
    while lines.len() < rows {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TuiState {
        TuiState::new("0.87.1", "/tmp/x/work", "0.0%/128k (auto)")
    }

    #[test]
    fn startup_screen_matches_the_reference_lines() {
        let mut s = state();
        s.context_files = vec!["AGENTS.md".to_string()];
        let screen = render(&s, 100, 30);
        assert_eq!(screen[1], " pi v0.87.1");
        assert_eq!(
            screen[2],
            " escape interrupt · ctrl+c/ctrl+d clear/exit · / commands · ! bash · ctrl+o more"
        );
        assert_eq!(
            screen[3],
            " Press ctrl+o to show full startup help and loaded resources."
        );
        assert_eq!(
            screen[5],
            " Pi can explain its own features and look up its docs. Ask it how to use or extend Pi."
        );
        // P7-6o(续4)/W0：`[Context]` 前补了 `:1776` 的 `Spacer(1)` 段落空行 ⇒ `[Context]` 落到第 8 行
        // （此前断言为第 7 行，属"缺该空行"的旧结构）。
        assert_eq!(screen[7], "");
        assert_eq!(screen[8], "[Context]");
        assert_eq!(screen[9], "  AGENTS.md");
        assert_eq!(screen.len(), 30);
        // 状态行紧跟在编辑器框之后（其后是补足行数的空行）⇒ 按内容定位，而不是取最后一行。
        // 状态区两行：cwd 行、然后 `status`（左）与模型名（右）
        let cwd_line = screen
            .iter()
            .find(|line| line.trim() == "/tmp/x/work")
            .expect("有 cwd 行");
        assert_eq!(cwd_line, "/tmp/x/work");
        let status = screen
            .iter()
            .find(|line| line.contains("0.0%/128k (auto)"))
            .expect("有状态行");
        assert!(status.starts_with("0.0%/128k (auto)"), "{status:?}");
    }

    #[test]
    fn editing_and_quit_follow_the_reference_semantics() {
        let mut s = state();
        assert_eq!(apply(&mut s, TuiEvent::Char('h')), TuiAction::Redraw);
        assert_eq!(apply(&mut s, TuiEvent::Char('i')), TuiAction::Redraw);
        assert_eq!(s.input, "hi");
        assert_eq!(apply(&mut s, TuiEvent::Backspace), TuiAction::Redraw);
        assert_eq!(s.input, "h");
        assert_eq!(
            apply(&mut s, TuiEvent::Enter),
            TuiAction::Submit("h".to_string())
        );
        assert_eq!(s.input, "");
        // 空编辑器 Ctrl+D ⇒ 退出（`keybindings.ts:95` 的语义）
        let mut s = state();
        assert_eq!(apply(&mut s, TuiEvent::CtrlD), TuiAction::Exit);
        assert!(s.exited);
        // 非空编辑器 Ctrl+D = `deleteCharForward`（ref `tui/src/keybindings.ts:120-122`）：
        // 删**光标处**的字符；光标在末尾时删不到东西（旧实现是 pop 末尾，已按 ref 更正）。
        let mut s = state();
        apply(&mut s, TuiEvent::Char('a'));
        apply(&mut s, TuiEvent::Char('b'));
        apply(&mut s, TuiEvent::CursorLeft); // 光标移到 "a|b"
        assert_eq!(apply(&mut s, TuiEvent::CtrlD), TuiAction::Redraw);
        assert!(!s.exited);
        assert_eq!(s.input, "a", "Ctrl+D 应删光标处字符（向后删）");
        // 光标已在末尾 ⇒ 无字符可删
        assert_eq!(apply(&mut s, TuiEvent::CtrlD), TuiAction::Redraw);
        assert_eq!(s.input, "a");
    }

    #[test]
    fn slash_quit_exits_and_plain_enter_ignores_empty_input() {
        let mut s = state();
        for ch in "/quit".chars() {
            apply(&mut s, TuiEvent::Char(ch));
        }
        assert_eq!(apply(&mut s, TuiEvent::Enter), TuiAction::Exit);
        assert!(s.exited);
        assert_eq!(s.input, "");

        let mut s = state();
        assert_eq!(
            apply(&mut s, TuiEvent::Enter),
            TuiAction::Redraw,
            "空输入回车不提交"
        );
        assert!(!s.exited);
    }

    #[test]
    fn clamp_handles_wide_characters() {
        assert_eq!(clamp_to_width("abc", 2), "ab");
        assert_eq!(clamp_to_width("中文", 3), "中");
        assert_eq!(clamp_to_width("中文", 4), "中文");
    }
}

/// 把 markdown 子集渲染成"消息区行"（**纯函数**）。
///
/// 输出**逐字对齐 `golden-tui/T05/run-1/snapshot.txt`** 的实测形态：
/// - 标题 `# 标题` ⇒ 一行 `" 标题"`（去掉 `#`，保留消息区的一个前导空格）；
/// - 列表项 `- 甲` ⇒ `" - 甲"`；
/// - 行内代码 `` `code` `` ⇒ 去掉反引号的 `code`；
/// - 代码块 ⇒ 围栏行 `" ```"`、内容行各加三个空格前缀、再一行 `" ```"`；
/// - 块与块之间插入一个空行。
///
/// 解析用 `pulldown-cmark`（P7 允许清单内，理由见 `notes/decisions.md`）；与 TS 的差异范围
/// 已在 `notes/reports/P7-4-spec.md` 列出（表格/删除线等扩展未启用；本函数只处理 T05 用到的子集）。
#[must_use]
pub fn render_markdown(input: &str) -> Vec<String> {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
    let mut out: Vec<String> = Vec::new();
    let mut inline = String::new();
    let mut in_list_item = false;
    let mut in_code_block = false;
    let mut code_line = String::new();
    let mut pending_blank = false;
    let flush_inline = |inline: &mut String,
                        in_list_item: bool,
                        out: &mut Vec<String>,
                        pending_blank: &mut bool| {
        let text = inline.trim_end().to_string();
        inline.clear();
        if text.is_empty() {
            return;
        }
        if *pending_blank && !out.is_empty() {
            out.push(String::new());
        }
        *pending_blank = false;
        out.push(if in_list_item {
            format!(" - {text}")
        } else {
            format!(" {text}")
        });
    };
    let _ = HeadingLevel::H1;
    for event in Parser::new_ext(input, Options::empty()) {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                if pending_blank && !out.is_empty() {
                    out.push(String::new());
                }
                pending_blank = false;
            }
            Event::End(TagEnd::Heading(_)) => {
                flush_inline(&mut inline, false, &mut out, &mut pending_blank);
                pending_blank = true;
            }
            Event::Start(Tag::List(_)) => {
                if pending_blank && !out.is_empty() {
                    out.push(String::new());
                }
                pending_blank = false;
            }
            Event::End(TagEnd::List(_)) => pending_blank = true,
            Event::Start(Tag::Item) => {
                in_list_item = true;
                pending_blank = false;
            }
            Event::End(TagEnd::Item) => {
                flush_inline(&mut inline, in_list_item, &mut out, &mut pending_blank);
                in_list_item = false;
            }
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                if pending_blank && !out.is_empty() {
                    out.push(String::new());
                }
                pending_blank = false;
                out.push(" ```".to_string());
            }
            Event::End(TagEnd::CodeBlock) => {
                if !code_line.is_empty() {
                    out.push(format!("   {}", code_line.trim_end()));
                    code_line.clear();
                }
                out.push(" ```".to_string());
                in_code_block = false;
                pending_blank = true;
            }
            Event::Text(text) => {
                if in_code_block {
                    for part in text.split('\n') {
                        if part.is_empty() {
                            if !code_line.is_empty() {
                                out.push(format!("   {}", code_line.trim_end()));
                                code_line.clear();
                            }
                        } else {
                            code_line.push_str(part);
                        }
                    }
                } else {
                    inline.push_str(&text);
                }
            }
            Event::Code(text) => inline.push_str(&text),
            Event::SoftBreak | Event::HardBreak => {
                if in_code_block {
                    if !code_line.is_empty() {
                        out.push(format!("   {}", code_line.trim_end()));
                        code_line.clear();
                    }
                } else {
                    inline.push(' ');
                }
            }
            Event::End(TagEnd::Paragraph) => {
                flush_inline(&mut inline, in_list_item, &mut out, &mut pending_blank);
                pending_blank = true;
            }
            other => {
                let _ = other;
            }
        }
    }
    flush_inline(&mut inline, in_list_item, &mut out, &mut pending_blank);
    while out.last().map(|line| line.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod markdown_tests {
    use super::{render_markdown, TranscriptItem, TuiState};

    #[test]
    fn streaming_deltas_merge_into_one_assistant_message() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        super::apply(
            &mut state,
            super::TuiEvent::AssistantDelta("# 标".to_string()),
        );
        super::apply(
            &mut state,
            super::TuiEvent::AssistantDelta("题\n".to_string()),
        );
        super::apply(
            &mut state,
            super::TuiEvent::AssistantDelta("正文".to_string()),
        );
        assert_eq!(state.transcript.len(), 1, "连续增量必须合并为一条助手消息");
        assert_eq!(
            state.transcript[0],
            TranscriptItem::Assistant("# 标题\n正文".to_string())
        );
        let text = super::render(&state, 100, 30).join("\n");
        assert!(
            text.contains(" 标题") && text.contains(" 正文"),
            "合并后按 markdown 渲染"
        );
    }

    #[test]
    fn usage_and_working_update_footer_and_status() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        super::apply(
            &mut state,
            super::TuiEvent::Usage {
                input: 10,
                output: 2,
                percent: Some(0.0),
                window: "128k".to_string(),
                auto: true,
                cost: None,
            },
        );
        assert_eq!(state.status, "↑10 ↓2 0.0%/128k (auto)");
        super::apply(
            &mut state,
            super::TuiEvent::Usage {
                input: 0,
                output: 0,
                percent: None,
                window: "128k".to_string(),
                auto: false,
                cost: None,
            },
        );
        // 随后一条 usage=0 的消息（`-c` 后新起一轮失败）**不得**把已有用量清零。
        assert_eq!(
            state.status, "↑10 ↓2 ?%/128k",
            "usage=0 不应清零已有用量；auto 关闭时不带 (auto)"
        );
        assert_eq!((state.usage_input, state.usage_output), (10, 2));
        super::apply(&mut state, super::TuiEvent::Working(true));
        assert!(state.working);
    }

    #[test]
    fn transcript_renders_user_assistant_and_tool_call_like_the_golden() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        state
            .transcript
            .push(TranscriptItem::User("say hi".to_string()));
        state.transcript.push(TranscriptItem::Assistant(
            "# 标题\n\n行内 `code` 与结尾。\n".to_string(),
        ));
        state.transcript.push(TranscriptItem::ToolCall {
            name: "read".to_string(),
            summary: "input.txt".to_string(),
        });
        let lines = super::render(&state, 100, 30);
        let text = lines.join("\n");
        assert!(text.contains(" say hi"), "用户消息行");
        assert!(text.contains(" 标题"), "助手 markdown 标题行");
        assert!(
            text.contains(" 行内 code 与结尾。"),
            "助手 markdown 段落行（行内代码去反引号）"
        );
        assert!(text.contains(" read input.txt"), "工具调用块只显示调用行");
        // 顺序：用户 → 助手 → 工具调用
        let pos_user = lines
            .iter()
            .position(|l| l == " say hi")
            .unwrap_or(usize::MAX);
        let pos_title = lines
            .iter()
            .position(|l| l == " 标题")
            .unwrap_or(usize::MAX);
        let pos_tool = lines
            .iter()
            .position(|l| l == " read input.txt")
            .unwrap_or(usize::MAX);
        assert!(
            pos_user < pos_title && pos_title < pos_tool,
            "会话区顺序必须与 golden 一致"
        );
    }

    #[test]
    fn markdown_subset_matches_the_t05_golden_shape() {
        let md = "# 标题\n\n- 甲\n- 乙\n\n行内 `code` 与结尾。\n\n```\nfn main() {}\n```\n";
        let lines = render_markdown(md);
        assert_eq!(
            lines,
            vec![
                " 标题".to_string(),
                String::new(),
                " - 甲".to_string(),
                " - 乙".to_string(),
                String::new(),
                " 行内 code 与结尾。".to_string(),
                String::new(),
                " ```".to_string(),
                "   fn main() {}".to_string(),
                " ```".to_string(),
            ]
        );
    }
}

#[cfg(test)]
mod editor_tests {
    use super::{apply, TuiAction, TuiEvent, TuiState};

    fn fresh() -> TuiState {
        TuiState::new("0.0.0", "work", "0.0%/128k (auto)")
    }

    fn send(state: &mut TuiState, event: TuiEvent) {
        let _ = apply(state, event);
    }

    fn type_text(state: &mut TuiState, text: &str) {
        for ch in text.chars() {
            send(state, TuiEvent::Char(ch));
        }
    }

    #[test]
    fn cursor_inserts_and_deletes_at_the_position() {
        let mut state = fresh();
        type_text(&mut state, "abcd");
        send(&mut state, TuiEvent::CursorLeft);
        send(&mut state, TuiEvent::CursorLeft);
        send(&mut state, TuiEvent::Char('X'));
        assert_eq!(state.input, "abXcd", "插入发生在光标处");
        assert_eq!(state.cursor, 3);
        send(&mut state, TuiEvent::Delete);
        assert_eq!(state.input, "abXd", "Delete 删光标右侧");
        send(&mut state, TuiEvent::Backspace);
        assert_eq!(state.input, "abd", "Backspace 删光标左侧");
        send(&mut state, TuiEvent::LineStart);
        assert_eq!(state.cursor, 0);
        send(&mut state, TuiEvent::LineEnd);
        assert_eq!(state.cursor, 3);
    }

    #[test]
    fn history_prev_next_walks_submitted_prompts() {
        let mut state = fresh();
        type_text(&mut state, "first");
        assert_eq!(
            apply(&mut state, TuiEvent::Enter),
            TuiAction::Submit("first".into())
        );
        type_text(&mut state, "second");
        assert_eq!(
            apply(&mut state, TuiEvent::Enter),
            TuiAction::Submit("second".into())
        );
        assert_eq!(
            state.history,
            vec!["first".to_string(), "second".to_string()]
        );
        send(&mut state, TuiEvent::HistoryPrev);
        assert_eq!(state.input, "second", "Up 先取最近一条");
        send(&mut state, TuiEvent::HistoryPrev);
        assert_eq!(state.input, "first", "再 Up 取更早一条");
        send(&mut state, TuiEvent::HistoryPrev);
        assert_eq!(state.input, "first", "已在最早一条时保持不动");
        send(&mut state, TuiEvent::HistoryNext);
        assert_eq!(state.input, "second", "Down 回到较近一条");
        send(&mut state, TuiEvent::HistoryNext);
        assert_eq!(state.input, "", "越过最新一条后清空编辑器");
    }

    #[test]
    fn ctrl_k_u_w_delete_the_expected_ranges() {
        let mut state = fresh();
        type_text(&mut state, "hello world");
        send(&mut state, TuiEvent::LineStart);
        for _ in 0..5 {
            // 光标移到 "hello|"（第 5 个字符之后）
            send(&mut state, TuiEvent::CursorRight);
        }
        send(&mut state, TuiEvent::DeleteToLineEnd);
        assert_eq!(state.input, "hello", "Ctrl+K 删到行尾");
        send(&mut state, TuiEvent::DeleteWordBackward);
        assert_eq!(
            state.input, "",
            "光标前没有空白 ⇒ Ctrl+W 删掉整个词（hello）"
        );
        send(&mut state, TuiEvent::DeleteToLineStart);
        assert_eq!(state.input, "", "Ctrl+U 删到行首");
    }

    #[test]
    fn wide_characters_are_deleted_as_whole_characters() {
        let mut state = fresh();
        type_text(&mut state, "中文🙂测");
        send(&mut state, TuiEvent::Backspace);
        assert_eq!(state.input, "中文🙂", "宽字符整体删除");
        send(&mut state, TuiEvent::LineStart);
        send(&mut state, TuiEvent::Delete);
        assert_eq!(state.input, "文🙂", "Delete 也整体删除");
    }

    #[test]
    fn paste_normalizes_cr_to_lf_and_does_not_submit() {
        let mut state = fresh();
        send(&mut state, TuiEvent::Paste("a\r\nb\rc".to_string()));
        assert_eq!(state.input, "a\nb\nc", "CR / CRLF 归一成 LF");
        assert_eq!(state.cursor, 5);
        // 粘贴不产生提交（返回 Redraw 而不是 Submit）。
        assert_eq!(
            apply(&mut state, TuiEvent::Paste("x".to_string())),
            TuiAction::Redraw
        );
    }
}

/// 命令面板的挂载位置（P7-6/W2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelPlacement {
    /// 画在**会话区**（编辑器上边框之上）：`/session`、`/hotkeys` 属此类（golden 实测）。
    Transcript,
    /// 画在**编辑器框之下、状态区之上**：`/settings`、`/model` 选择器属此类（golden 实测）。
    BelowEditor,
}

impl TuiState {
    /// 打开一个命令面板（由 pi-cli 传入已渲染好的行）。
    pub fn open_panel(&mut self, placement: PanelPlacement, lines: Vec<String>) {
        self.panel = Some((placement, lines));
    }

    /// 关闭当前面板（Esc / 提交 / resize 等）。
    pub fn close_panel(&mut self) {
        self.panel = None;
    }
}

/// 已知内建命令表（P7-6/W1 取证：`core/slash-commands.ts:20-43` 的 24 条，去掉本批后移/排除的 8 条
/// 后仍有分派意义者）。**纯数据 + 纯函数**：pi-cli 只按返回值决定执行。
/// 不在表里的 `/xxx` 一律按普通输入提交（T18 实测：未知命令会被当 prompt 发给模型）。
#[must_use]
pub fn builtin_command(name: &str) -> bool {
    const COMMANDS: [&str; 24] = [
        "settings",
        "model",
        "tree",
        "thinking",
        "scoped-models",
        "export",
        "import",
        "share",
        "bug",
        "copy",
        "name",
        "session",
        "changelog",
        "hotkeys",
        "fork",
        "clone",
        "trust",
        "login",
        "logout",
        "new",
        "compact",
        "resume",
        "reload",
        "quit",
    ];
    COMMANDS.contains(&name)
}

#[cfg(test)]
mod panel_tests {
    use super::{render, render_all, PanelPlacement, TuiState};

    fn session_panel() -> Vec<String> {
        vec![
            " Session Info".to_string(),
            String::new(),
            " File: /tmp/sess/x.jsonl".to_string(),
            " ID: abc".to_string(),
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
        ]
    }

    #[test]
    fn render_all_keeps_the_whole_panel_and_context_line() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        state.context_files = vec!["AGENTS.md".to_string()];
        state.open_panel(PanelPlacement::Transcript, session_panel());
        let all = render_all(&state, 100, 30);
        let capped = render(&state, 100, 30);
        let text = all.join("\n");
        // 完整列表里必须同时出现 `[Context]` 行与面板的 20 行。
        assert!(text.contains("[Context]"), "完整列表缺 [Context] 行");
        for line in session_panel() {
            if line.is_empty() {
                continue;
            }
            assert!(text.contains(&line), "完整列表缺面板行：{line:?}");
        }
        assert_eq!(session_panel().len(), 19, "面板应有 19 行（含 5 个空行）");
        // 内容超过一屏时：完整列表比裁剪后的屏幕长，且裁剪只丢顶部（`[Context]` 可能被挤出）。
        assert!(
            all.len() > capped.len(),
            "完整列表({}) 应长于裁剪后({})",
            all.len(),
            capped.len()
        );
        assert!(
            text.contains("Status: Inactive"),
            "完整列表应包含面板最后一行"
        );
    }
}

// ==== P7-6b/W1：三个命令面板的**逐字数据**（来源：golden-tui/T15、T17、T16 的 rows=60 全量渲染，
// 见 notes/reports/P7-6b-panel-dumps.txt）。纯数据常量，渲染时原样输出。
/// 来源：golden 场景 `T15`（录制 cols=100），由 `tools/difftest/transcribe-panels.mjs` 转录。
/// 逐行含空行与结构行；空行游程与 golden 区间完全一致（脚本断言）。
/// 其它终端宽度下的换行/裁剪**未覆盖**（登记为已知限制）。
/// `/hotkeys` 面板在给定列宽下的行（**动态渲染**，#15 部分收口）。
///
/// 行构成与旧常量 [`HOTKEYS_LINES`] 一致：`[header, ""] + Markdown 块`（无尾部 paddingY 空行）。
///
/// **宽度选择规则**（不静默回退到 100）：
/// - 精确命中 60/80/100/120 ⇒ 用该档；
/// - 其它宽度 ⇒ **分档到不大于 `cols` 的最大档**（低于 60 则用 60 档）。
///   分档是**已知限制**（登记在 #15 名下），不是"最近似"猜测。
#[must_use]
pub fn hotkeys_panel_for_width(cols: usize) -> Vec<String> {
    let table = hotkeys_widths::HOTKEYS_PANEL_BY_WIDTH;
    let mut chosen: &[&str] = table.first().map_or(&[], |(_, lines)| *lines);
    for (width, lines) in table {
        if *width <= cols {
            chosen = lines;
        }
    }
    chosen.iter().map(|line| (*line).to_string()).collect()
}

/// 内置斜杠命令（取自 ref `core/slash-commands.ts:19-44` 的 `BUILTIN_SLASH_COMMANDS`，24 项）。
const SLASH_COMMANDS: [(&str, &str); 24] = [
    ("settings", "Open settings menu"),
    (
        "model",
        "<provider/model> — Select model (opens selector UI)",
    ),
    ("tree", "Navigate session tree (switch branches)"),
    ("thinking", "<level> — Set thinking level"),
    ("scoped-models", "Enable/disable models for Ctrl+P cycling"),
    ("export", "Export session"),
    ("import", "Import and resume a session from a JSONL file"),
    ("share", "Share session as a secret GitHub gist"),
    ("bug", "Report a bug to the Pi developers"),
    ("copy", "Copy last agent message to clipboard"),
    ("name", "Set session display name"),
    ("session", "Show session info and stats"),
    ("changelog", "Show changelog entries"),
    ("hotkeys", "Show all keyboard shortcuts"),
    ("fork", "Create a new fork from a previous user message"),
    (
        "clone",
        "Duplicate the current session at the current position",
    ),
    ("trust", "Save project trust decision for future sessions"),
    ("login", "Configure provider authentication"),
    ("logout", "Remove provider authentication"),
    ("new", "Start a new session"),
    ("compact", "Manually compact the session context"),
    ("resume", "Resume a different session"),
    (
        "reload",
        "Reload keybindings, extensions, skills, prompts, theme",
    ),
    ("quit", "Quit"),
];

impl TuiState {
    /// 把输入里的粘贴占位符展开成原文（ref `editor.ts:1090-1096` 的 `getExpandedText`）。
    #[must_use]
    pub fn expanded_text(&self) -> String {
        expand_paste_markers(&self.pastes, &self.input)
    }
}

/// 光标前是否紧邻一个**已登记的**粘贴占位符；返回 `(起点, 终点)`（字符下标）。
fn marker_before_cursor(state: &TuiState) -> Option<(usize, usize)> {
    marker_spans(state)
        .into_iter()
        .find(|(_, end)| *end == state.cursor)
}

/// 输入里所有已登记占位符的字符区间（`[start, end)`）。
fn marker_spans(state: &TuiState) -> Vec<(usize, usize)> {
    let chars: Vec<char> = state.input.chars().collect();
    let mut spans = Vec::new();
    let mut index = 0;
    while index + 8 < chars.len() {
        if chars[index] == '[' {
            let rest: String = chars[index..].iter().collect();
            if let Some(length) = marker_length(&rest, &state.pastes) {
                spans.push((index, index + length));
                index += length;
                continue;
            }
        }
        index += 1;
    }
    spans
}

/// 读 `input[start..]` 处占位符的编号。
fn marker_id_at(input: &str, start: usize) -> Option<u32> {
    let rest: String = input.chars().skip(start).collect();
    let after = rest.strip_prefix("[paste #")?;
    let digits_end = after.find(|ch: char| !ch.is_ascii_digit())?;
    after[..digits_end].parse().ok()
}

/// `text` 是否以合法占位符开头；是则返回它的字符长度（ref `PASTE_MARKER_REGEX`）。
fn marker_length(text: &str, pastes: &std::collections::BTreeMap<u32, String>) -> Option<usize> {
    let rest = text.strip_prefix("[paste #")?;
    let digits_end = rest.find(|ch: char| !ch.is_ascii_digit())?;
    let id: u32 = rest[..digits_end].parse().ok()?;
    pastes.get(&id)?;
    let tail = &rest[digits_end..];
    if let Some(inner) = tail.strip_prefix(" +") {
        let count_end = inner.find(|ch: char| !ch.is_ascii_digit())?;
        let suffix = inner[count_end..].strip_prefix(" lines]")?;
        let consumed = 8 + digits_end + 2 + count_end + " lines]".len();
        let _ = suffix;
        return Some(consumed);
    }
    let inner = tail.strip_prefix(' ')?;
    let count_end = inner.find(|ch: char| !ch.is_ascii_digit())?;
    let _suffix = inner[count_end..].strip_prefix(" chars]")?;
    Some(8 + digits_end + 1 + count_end + " chars]".len())
}

/// 把输入里的占位符换回原文（ref `expandPasteMarkers`）。
fn expand_paste_markers(pastes: &std::collections::BTreeMap<u32, String>, input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '[' {
            let rest: String = chars[index..].iter().collect();
            if let Some(length) = marker_length(&rest, pastes) {
                let marker: String = chars[index..index + length].iter().collect();
                let id_start = marker.find('#').map_or(0, |pos| pos + 1);
                let id_end = marker[id_start..]
                    .find(|ch: char| !ch.is_ascii_digit())
                    .map_or(marker.len(), |offset| id_start + offset);
                if let Ok(id) = marker[id_start..id_end].parse::<u32>() {
                    if let Some(content) = pastes.get(&id) {
                        out.push_str(content);
                        index += length;
                        continue;
                    }
                }
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// 删掉一个占位符：注销登记、把更高编号整体下移，并**重写文本里的编号**
/// （ref `editor.ts:1394-1415`：`[paste #3]` 在 `[paste #1]` 被删后变成 `[paste #2]`）。
fn drop_paste_marker(state: &mut TuiState, id: u32, start: usize, end: usize) {
    remove_range(state, start, end);
    // 先按**语法**重写文本里的编号（此时登记表还没动，不能用 `marker_length` 的登记校验）。
    let chars: Vec<char> = state.input.chars().collect();
    let mut rebuilt = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '[' {
            let rest: String = chars[index..].iter().collect();
            if let Some((length, found)) = marker_syntax(&rest) {
                let marker: String = chars[index..index + length].iter().collect();
                if found > id {
                    // `[paste #<id><suffix>` —— 只改编号，后缀原样保留。
                    let digits = marker
                        .trim_start_matches("[paste #")
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .count();
                    let suffix = &marker["[paste #".len() + digits..];
                    rebuilt.push_str(&format!("[paste #{}{suffix}", found - 1));
                } else {
                    rebuilt.push_str(&marker);
                }
                index += length;
                continue;
            }
        }
        rebuilt.push(chars[index]);
        index += 1;
    }
    state.input = rebuilt;
    // 登记表：注销并整体下移（ref `editor.ts:1394-1408`）。
    state.pastes.remove(&id);
    let higher: Vec<u32> = state
        .pastes
        .keys()
        .copied()
        .filter(|key| *key > id)
        .collect();
    for key in higher {
        if let Some(content) = state.pastes.get(&key).cloned() {
            state.pastes.insert(key - 1, content);
            state.pastes.remove(&key);
        }
    }
    state.paste_counter = state.paste_counter.saturating_sub(1);
}

/// 只看**语法**的占位符解析：返回 `(字符长度, 编号)`（不校验编号是否已登记）。
fn marker_syntax(text: &str) -> Option<(usize, u32)> {
    let rest = text.strip_prefix("[paste #")?;
    let digits_end = rest.find(|ch: char| !ch.is_ascii_digit())?;
    let id: u32 = rest[..digits_end].parse().ok()?;
    let tail = &rest[digits_end..];
    if let Some(inner) = tail.strip_prefix(" +") {
        let count_end = inner.find(|ch: char| !ch.is_ascii_digit())?;
        inner[count_end..].strip_prefix(" lines]")?;
        return Some((8 + digits_end + 2 + count_end + " lines]".len(), id));
    }
    let inner = tail.strip_prefix(' ')?;
    let count_end = inner.find(|ch: char| !ch.is_ascii_digit())?;
    inner[count_end..].strip_prefix(" chars]")?;
    Some((8 + digits_end + 1 + count_end + " chars]".len(), id))
}

/// 删除 `[start, end)` 字符区间。
fn remove_range(state: &mut TuiState, start: usize, end: usize) {
    let mut chars: Vec<char> = state.input.chars().collect();
    if start < end && end <= chars.len() {
        chars.drain(start..end);
        state.input = chars.into_iter().collect();
        state.cursor = start;
    }
}

/// 按当前输入重算补全菜单开关（`should_open_completion` 是纯函数，这里只写入状态）。
fn refresh_completion(state: &mut TuiState) {
    state.completion_open = should_open_completion(&state.input);
    state.completion_selected = 0;
}

/// 是否显示斜杠命令补全菜单。
///
/// 规则（由 golden 反推）：输入以 `/` 开头、不含空格、**有匹配项**，且
/// 输入**尚未精确等于**某个命令名——否则像 `/session` 这样已经打全的命令，
/// Enter 应该是"提交命令"而不是"接受补全"（T04/T18 的 golden 依赖这一点）。
#[must_use]
fn should_open_completion(input: &str) -> bool {
    if !input.starts_with('/') || input.contains(' ') {
        return false;
    }
    // TS 在 `/tree`（**恰好等于命令名**）时菜单仍然可见（`golden-tui/T68-completion-backspace`
    // 帧 `after-tree` 里 `→ tree …` 那行在），所以这里不再按"精确匹配"关闭；
    // "Enter 该提交还是该接受补全"由 Enter 分支自己判断（见 `accepts_nothing`）。
    !completion_matches(input).is_empty()
}

/// 按输入过滤斜杠命令：前缀匹配优先，其次包含匹配（ref 的 fuzzy 过滤的保守子集）。
#[must_use]
pub fn completion_matches(input: &str) -> Vec<(&'static str, &'static str)> {
    let query = input.trim_start_matches('/').to_ascii_lowercase();
    if query.is_empty() {
        return SLASH_COMMANDS.to_vec();
    }
    let mut prefix: Vec<(&'static str, &'static str)> = SLASH_COMMANDS
        .iter()
        .copied()
        .filter(|(name, _)| name.to_ascii_lowercase().starts_with(&query))
        .collect();
    let mut contains: Vec<(&'static str, &'static str)> = SLASH_COMMANDS
        .iter()
        .copied()
        .filter(|(name, _)| {
            let lower = name.to_ascii_lowercase();
            !lower.starts_with(&query) && lower.contains(&query)
        })
        .collect();
    // 子串匹配按**匹配位置**排序（T64 golden：`/s` 下 `resume`（s 在第 3 个字符）排在
    // `hotkeys`（s 在第 7 个字符）之前），位置相同再按名字长短/字典序，保证稳定。
    contains.sort_by_key(|(name, _)| {
        let lower = name.to_ascii_lowercase();
        let at = lower.find(&query).unwrap_or(usize::MAX);
        (at, lower.len(), lower)
    });
    prefix.extend(contains);
    prefix
}

pub const HOTKEYS_LINES: &[&str] = &[
    " Keyboard Shortcuts",
    "",
    "",
    " Navigation",
    "",
    " ┌───────────────────────────────────────────────────────┬──────────────────────────────┐",
    " │ Key                                                   │ Action                       │",
    " ├───────────────────────────────────────────────────────┼──────────────────────────────┤",
    " │ Up / Down / Left/Ctrl+B / Right/Ctrl+F                │ Move cursor / browse history │",
    " ├───────────────────────────────────────────────────────┼──────────────────────────────┤",
    " │ Alt+Left/Ctrl+Left/Alt+B / Alt+Right/Ctrl+Right/Alt+F │ Move by word                 │",
    " ├───────────────────────────────────────────────────────┼──────────────────────────────┤",
    " │ Home/Ctrl+Home/Ctrl+A                                 │ Start of line                │",
    " ├───────────────────────────────────────────────────────┼──────────────────────────────┤",
    " │ End/Ctrl+End/Ctrl+E                                   │ End of line                  │",
    " ├───────────────────────────────────────────────────────┼──────────────────────────────┤",
    " │ Ctrl+]                                                │ Jump forward to character    │",
    " ├───────────────────────────────────────────────────────┼──────────────────────────────┤",
    " │ Ctrl+Alt+]                                            │ Jump backward to character   │",
    " ├───────────────────────────────────────────────────────┼──────────────────────────────┤",
    " │ PageUp/Ctrl+PageUp / PageDown/Ctrl+PageDown           │ Scroll by page               │",
    " └───────────────────────────────────────────────────────┴──────────────────────────────┘",
    "",
    " Editing",
    "",
    " ┌──────────────────────┬──────────────────────────────────────────────┐",
    " │ Key                  │ Action                                       │",
    " ├──────────────────────┼──────────────────────────────────────────────┤",
    " │ Enter                │ Send message                                 │",
    " ├──────────────────────┼──────────────────────────────────────────────┤",
    " │ Shift+Enter/Ctrl+J   │ New line                                     │",
    " ├──────────────────────┼──────────────────────────────────────────────┤",
    " │ Ctrl+W/Alt+Backspace │ Delete word backwards                        │",
    " ├──────────────────────┼──────────────────────────────────────────────┤",
    " │ Alt+D/Alt+Delete     │ Delete word forwards                         │",
    " ├──────────────────────┼──────────────────────────────────────────────┤",
    " │ Ctrl+U               │ Delete to start of line                      │",
    " ├──────────────────────┼──────────────────────────────────────────────┤",
    " │ Ctrl+K               │ Delete to end of line                        │",
    " ├──────────────────────┼──────────────────────────────────────────────┤",
    " │ Ctrl+Y               │ Paste the most-recently-deleted text         │",
    " ├──────────────────────┼──────────────────────────────────────────────┤",
    " │ Alt+Y                │ Cycle through the deleted text after pasting │",
    " ├──────────────────────┼──────────────────────────────────────────────┤",
    " │ Ctrl+-               │ Undo                                         │",
    " └──────────────────────┴──────────────────────────────────────────────┘",
    "",
    " Other",
    "",
    " ┌───────────────────────┬──────────────────────────────────────────┐",
    " │ Key                   │ Action                                   │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Tab                   │ Path completion / accept autocomplete    │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Escape                │ Cancel autocomplete / abort streaming    │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+C                │ Clear editor (first) / exit (second)     │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+D                │ Exit (when editor is empty)              │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+Z                │ Suspend to background                    │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Shift+Tab             │ Cycle thinking level                     │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+P / Shift+Ctrl+P │ Cycle models                             │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+L                │ Open model selector                      │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+O                │ Toggle tool output expansion             │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+T                │ Toggle thinking block visibility         │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+G                │ Edit message in external editor          │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+X                │ Copy selection or last assistant message │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Alt+Enter             │ Queue follow-up message                  │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Alt+Up                │ Restore queued messages                  │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ Ctrl+V                │ Paste image or text from clipboard       │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ /                     │ Slash commands                           │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ !                     │ Run bash command                         │",
    " ├───────────────────────┼──────────────────────────────────────────┤",
    " │ !!                    │ Run bash command (excluded from context) │",
    " └───────────────────────┴──────────────────────────────────────────┘",
];
/// 来源：golden 场景 `T17`（录制 cols=100），由 `tools/difftest/transcribe-panels.mjs` 转录。
/// 逐行含空行与结构行；空行游程与 golden 区间完全一致（脚本断言）。
/// 其它终端宽度下的换行/裁剪**未覆盖**（登记为已知限制）。
pub const SETTINGS_LINES: &[&str] = &[
    ">",
    "",
    "→ Auto-compact                      true",
    "  Auto-resize images                true",
    "  Block images                      false",
    "  Skill commands                    true",
    "  Show hardware cursor              false",
    "  Editor padding                    0",
    "  Output padding                    1",
    "  Autocomplete max items            5",
    "  Clear on shrink                   false",
    "  Terminal progress                 false",
    "  (1/31)",
    "",
    "  Automatically compact context when it gets too large",
    "",
    "  Type to search · Enter/Space to change · Esc to cancel",
];
/// 来源：golden 场景 `T16`（录制 cols=100），由 `tools/difftest/transcribe-panels.mjs` 转录。
/// 逐行含空行与结构行；空行游程与 golden 区间完全一致（脚本断言）。
/// 其它终端宽度下的换行/裁剪**未覆盖**（登记为已知限制）。
pub const MODEL_LINES: &[&str] = &[
    "",
    "Only showing models from configured providers. Use /login to add providers.",
    "",
    ">",
    "",
    "→ ✓ mock-model [openai-completions] · default",
    "",
    "  Model Name: mock-model",
    "",
    "  Model catalogs refreshed.",
    "",
    "  Enter to select · Ctrl+S to set as default · Escape/Ctrl+C to cancel",
];

#[cfg(test)]
mod command_tests {
    use super::{apply, builtin_command, TuiAction, TuiEvent, TuiState};

    /// `core/slash-commands.ts:20-43` 的 24 条内建命令（逐条列举，缺一即失败）。
    const ALL_24: [&str; 24] = [
        "settings",
        "model",
        "tree",
        "thinking",
        "scoped-models",
        "export",
        "import",
        "share",
        "bug",
        "copy",
        "name",
        "session",
        "changelog",
        "hotkeys",
        "fork",
        "clone",
        "trust",
        "login",
        "logout",
        "new",
        "compact",
        "resume",
        "reload",
        "quit",
    ];

    #[test]
    fn every_builtin_command_is_recognised() {
        assert_eq!(ALL_24.len(), 24, "内建命令必须是 24 条");
        for name in ALL_24 {
            assert!(builtin_command(name), "内建命令未被识别：{name}");
        }
        // 负向对照：非内建命令与大小写变体都不得被识别（大小写敏感）。
        for name in ["xyz", "Settings", "hotkey", "", "session ", "help"] {
            assert!(!builtin_command(name), "不应被识别为内建命令：{name:?}");
        }
    }

    #[test]
    fn enter_dispatches_builtin_commands_and_submits_unknown() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        for ch in "/session".chars() {
            let _ = apply(&mut state, TuiEvent::Char(ch));
        }
        assert_eq!(
            apply(&mut state, TuiEvent::Enter),
            TuiAction::Command("session".to_string()),
            "已知命令应走 Command"
        );
        // 未知 `/xyz` 与普通文本都走 Submit（T18 实测：未知命令被当作 prompt）。
        for ch in "/xyz".chars() {
            let _ = apply(&mut state, TuiEvent::Char(ch));
        }
        assert_eq!(
            apply(&mut state, TuiEvent::Enter),
            TuiAction::Submit("/xyz".to_string())
        );
        for ch in "hello".chars() {
            let _ = apply(&mut state, TuiEvent::Char(ch));
        }
        assert_eq!(
            apply(&mut state, TuiEvent::Enter),
            TuiAction::Submit("hello".to_string())
        );
    }
}

/// `/model` 选择器的一行（数据化后才能在方向键移动时重绘 `→ ` 标记）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRow {
    /// 行前缀里 `→ ` 之后的内容（选中标记由渲染时决定）。
    pub text: String,
}

/// 把预渲染的 `/model` 面板行转成可选中的行（去掉行首的 `→ `/`  ` 标记与 `>` 提示行）。
#[must_use]
pub fn model_rows_from_lines(lines: &[&str]) -> Vec<ModelRow> {
    lines
        .iter()
        .filter(|line| !line.trim_start().starts_with('>'))
        .map(|line| {
            let stripped = line
                .strip_prefix("→ ")
                .or_else(|| line.strip_prefix("  "))
                .unwrap_or(line);
            ModelRow {
                text: stripped.to_string(),
            }
        })
        .collect()
}

/// 渲染 `/model` 选择器面板：`selected` 那一行带 `→ `，其余带 `  `，`hint` 放在最后。
#[must_use]
pub fn render_model_panel(rows: &[ModelRow], selected: usize, hint: &str) -> Vec<String> {
    let mut lines = Vec::with_capacity(rows.len() + 1);
    for (index, row) in rows.iter().enumerate() {
        let marker = if index == selected { "→ " } else { "  " };
        lines.push(format!("{marker}{}", row.text));
    }
    lines.push(format!("  {hint}"));
    lines
}

#[cfg(test)]
mod panel_blank_tests {
    use super::{render_all, PanelPlacement, TuiState};

    /// 读取 `crates/pi-tui/tests/fixtures/panels-golden.txt`（无依赖文本夹具，与 `.json` 同源）。
    fn fixture(case: &str) -> Vec<String> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/panels-golden.txt");
        let text = std::fs::read_to_string(path).expect("读取面板夹具失败");
        let mut in_case = false;
        let mut lines = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("case ") {
                in_case = rest.split(' ').next() == Some(case);
                continue;
            }
            if !in_case {
                continue;
            }
            if let Some(b64) = line.strip_prefix("line ") {
                const TABLE: &[u8; 64] =
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
                let mut out = Vec::new();
                let (mut acc, mut bits) = (0u32, 0u32);
                for byte in b64.bytes() {
                    let Some(index) = TABLE.iter().position(|&c| c == byte) else {
                        continue;
                    };
                    acc = (acc << 6) | index as u32;
                    bits += 6;
                    if bits >= 8 {
                        bits -= 8;
                        out.push(((acc >> bits) & 0xff) as u8);
                    }
                }
                lines.push(String::from_utf8(out).expect("夹具行不是 UTF-8"));
            }
        }
        lines
    }

    /// T16 `/model`：`render_all` 的「编辑器上边框 + 空输入行 + 面板 11 行」必须与 golden 逐行（含空行）一致。
    #[test]
    fn model_panel_region_matches_golden() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        state.context_files = vec!["AGENTS.md".to_string()];
        let panel: Vec<String> = super::MODEL_LINES
            .iter()
            .map(|line| (*line).to_string())
            .collect();
        state.open_panel(PanelPlacement::BelowEditor, panel);
        let all = render_all(&state, 100, 30);
        let golden = fixture("T16");
        assert!(!golden.is_empty(), "T16 夹具为空");
        // golden 区间 = 上边框、空输入行、面板 11 行 ⇒ 取 render_all 末尾同样长度的一段对齐。
        let take = golden.len();
        assert!(
            all.len() >= take,
            "render_all 行数不足：{} < {}",
            all.len(),
            take
        );
        // 去掉尾部（cwd/usage/补白）后再对齐：用面板最后一行的内容定位。
        let anchor = golden.last().expect("夹具末行").clone();
        let end = all
            .iter()
            .rposition(|line| line.trim_end() == anchor.trim_end())
            .expect("找不到面板末行");
        let sliced: Vec<String> = all[end + 1 - take..=end]
            .iter()
            .map(|l| l.trim_end().to_string())
            .collect();
        let expected: Vec<String> = golden.iter().map(|l| l.trim_end().to_string()).collect();
        assert_eq!(
            sliced, expected,
            "T16 面板区间与 golden 不一致\n实际={sliced:?}\n期望={expected:?}"
        );
    }
}

#[cfg(test)]
mod panel_width_tests {
    use super::{render_all, PanelPlacement, TuiState};
    use crate::diff::{diff_render, DiffState};

    fn state_with(panel: &[&str]) -> TuiState {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        state.context_files = vec!["AGENTS.md".to_string()];
        state.open_panel(
            PanelPlacement::BelowEditor,
            panel.iter().map(|s| (*s).to_string()).collect(),
        );
        state
    }

    /// `cols=60/80` 下：`render_all` 必须把行 clamp 到列宽内（不产生超宽行），因此 `diff_render` 不返回 Err。
    /// 依据：ref 的超宽检查只在**增量路径**（`tui-main-screen.ts:534-548` 抛错），首次整屏渲染不检查；
    /// pi-rs 的 `render_all` 用 `clamp_to_width` 截断 ⇒ 两条路径都不会出现超宽行。
    #[test]
    fn panels_are_clamped_and_never_error_at_narrow_widths() {
        for cols in [60usize, 80] {
            for panel in [
                super::HOTKEYS_LINES,
                super::SETTINGS_LINES,
                super::MODEL_LINES,
            ] {
                let state = state_with(panel);
                let lines = render_all(&state, cols, 30);
                let over: Vec<(usize, usize)> = lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| unicode_width::UnicodeWidthStr::width(line.as_str()) > cols)
                    .map(|(i, line)| (i, unicode_width::UnicodeWidthStr::width(line.as_str())))
                    .collect();
                assert!(over.is_empty(), "cols={cols} 出现超宽行：{over:?}");
                let mut diff_state = DiffState::new();
                let result = diff_render(&mut diff_state, &lines, cols, 30);
                assert!(
                    result.is_ok(),
                    "cols={cols} 时 diff_render 返回 Err：{:?}",
                    result.err()
                );
            }
        }
    }
}

#[cfg(test)]
mod settings_panel_tests {
    use super::{render_all, PanelPlacement, TuiState};

    fn fixture_lines(case: &str) -> Vec<String> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/panels-golden.txt");
        let text = std::fs::read_to_string(path).expect("读取面板夹具失败");
        let mut inside = false;
        let mut out = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("case ") {
                inside = rest.split(' ').next() == Some(case);
                continue;
            }
            if !inside {
                continue;
            }
            if let Some(b64) = line.strip_prefix("line ") {
                const TABLE: &[u8; 64] =
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
                let mut bytes = Vec::new();
                let (mut acc, mut bits) = (0u32, 0u32);
                for byte in b64.bytes() {
                    let Some(index) = TABLE.iter().position(|&c| c == byte) else {
                        continue;
                    };
                    acc = (acc << 6) | index as u32;
                    bits += 6;
                    if bits >= 8 {
                        bits -= 8;
                        bytes.push(((acc >> bits) & 0xff) as u8);
                    }
                }
                out.push(String::from_utf8(bytes).expect("夹具行不是 UTF-8"));
            }
        }
        out
    }

    /// T17 `/settings`：`render_all` 的「上边框 + 面板 17 行」必须与 golden 帧0 行 9..26 逐行（含空行）一致。
    #[test]
    fn settings_panel_region_matches_golden() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        state.context_files = vec!["AGENTS.md".to_string()];
        state.open_panel(
            PanelPlacement::BelowEditor,
            super::SETTINGS_LINES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        );
        let all = render_all(&state, 100, 30);
        let golden: Vec<String> = fixture_lines("T17")
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .collect();
        assert!(!golden.is_empty(), "T17 夹具为空");
        let take = golden.len();
        let anchor = golden.last().expect("夹具末行").clone();
        let end = all
            .iter()
            .rposition(|line| line.trim_end() == anchor.trim_end())
            .expect("找不到面板末行（settings 提示行）");
        assert!(
            end + 1 >= take,
            "render_all 行数不足：{} < {}",
            end + 1,
            take
        );
        let actual: Vec<String> = all[end + 1 - take..=end]
            .iter()
            .map(|l| l.trim_end().to_string())
            .collect();
        assert_eq!(
            actual, golden,
            "T17 面板区间与 golden 不一致\n实际={actual:?}\n期望={golden:?}"
        );
    }
}

#[cfg(test)]
mod t05_tail_tests {
    use super::{render_all, TranscriptItem, TuiState};

    fn state_with(items: Vec<TranscriptItem>) -> TuiState {
        let mut state = TuiState::new("0.0.0", "work", "1.1%/128k (auto)");
        state.context_files = vec!["AGENTS.md".to_string()];
        state.transcript = items;
        state
    }

    fn assistant_markdown() -> String {
        "# 标题\n\n- 甲\n- 乙\n\n行内 `code` 与结尾。\n\n```\nfn main() {}\n```".to_string()
    }

    /// P7-6o(续3)/W2.1：把 r3 在 T05 末帧会喂给 `diff_render` 的 lines 数组**末尾 8 行**（保留空行）打出来，
    /// 与 golden 的末尾做对照；T01 作为对照（末条为单行文本）。
    #[test]
    fn tail_lines_are_visible_for_t05_and_t01() {
        let t05 = state_with(vec![
            TranscriptItem::User("say hi".to_string()),
            TranscriptItem::Assistant(assistant_markdown()),
        ]);
        let t01 = state_with(vec![
            TranscriptItem::User("Hi".to_string()),
            TranscriptItem::Assistant("Hello world".to_string()),
        ]);
        for (name, state) in [("T05", &t05), ("T01", &t01)] {
            let lines = render_all(state, 100, 30);
            let tail: Vec<&String> = lines.iter().rev().take(8).collect();
            println!(
                "{name} 末尾 8 行（倒序输出后再反转）: {:?}",
                tail.iter().rev().collect::<Vec<_>>()
            );
            let trailing = lines.iter().rev().take_while(|l| l.is_empty()).count();
            println!("{name}: 总行数={} 末尾连续空行={trailing}", lines.len());
            // 断言：行数组末尾不是空行（末尾空行属屏幕缓冲，不属内容层）。
            assert_eq!(trailing, 0, "{name} 的行数组不应以空行结尾");
        }
    }
}

/// 在 `render_all` 的结果中、**编辑器光标处**插入 `CURSOR_MARKER`（仅供 r3 使用）。
///
/// ref：编辑器组件在有焦点时把 `CURSOR_MARKER`（`crates/pi-tui/src/diff.rs` 的
/// `CURSOR_MARKER = "\x1b_pi:c\x07"`，等价 `tui.ts:168`）插到行内光标位置；`TuiMainScreen`
/// 在 `extractCursorPosition`（`tui.ts:1382-1400`）里把它解析成 `(row, col)` 并从行里剥除。
/// 这里按同一规则：找到**编辑器输入行**（下边框之上的最后一段），在 `state.cursor` 指定的字符列插入标记。
/// **R1/R2 不调用本函数**（保证 R1 输出逐字节不变）。
#[must_use]
pub fn render_all_marker(state: &TuiState, cols: usize, rows: usize) -> Vec<String> {
    render_all_marker_impl(state, cols, rows, None)
}

/// 同 [`render_all_marker`]，但复用 [`RenderCache`]（输出必须与未缓存版本逐字节一致）。
pub fn render_all_marker_cached(
    state: &TuiState,
    cols: usize,
    rows: usize,
    cache: &mut RenderCache,
) -> Vec<String> {
    render_all_marker_impl(state, cols, rows, Some(cache))
}

/// 两者的共同实现：先渲染，再在硬件光标处插入 `CURSOR_MARKER`。
fn render_all_marker_impl(
    state: &TuiState,
    cols: usize,
    rows: usize,
    cache: Option<&mut RenderCache>,
) -> Vec<String> {
    let mut lines = render_all_cached_inner(state, cols, rows, cache);
    let border = "─".repeat(cols);
    // 下边框 = 从末尾往前第一个整宽边框；它之上是输入行（多行输入时取最后一行）。
    let Some(bottom) = lines.iter().rposition(|line| line == &border) else {
        return lines;
    };
    if bottom == 0 {
        return lines;
    }
    let target = bottom - 1;
    // 光标列：`state.cursor` 是**字符**下标；本行为输入的最后一段时，列即该段的字符偏移。
    let last_segment_start = state.input.rfind('\n').map_or(0, |i| i + 1);
    let col = state.cursor.saturating_sub(last_segment_start);
    let line = lines[target].clone();
    let byte_index = line
        .char_indices()
        .nth(col)
        .map_or(line.len(), |(index, _)| index);
    // ref `packages/tui/src/components/editor.ts:575-600`：`showHardwareCursor` 默认 false 时，
    // 编辑器用**反显字符**画"软件光标"（行尾时是反显空格）。我们此前只插了硬件光标标记、
    // 而标记本身不显示，于是真机上"没有光标、无法定位"。
    let with_marker = if let Some(current) = line[byte_index..].chars().next() {
        let mut out = String::with_capacity(line.len() + 16);
        out.push_str(&line[..byte_index]);
        out.push_str(crate::diff::CURSOR_MARKER);
        out.push_str("\x1b[7m");
        out.push(current);
        out.push_str("\x1b[0m");
        out.push_str(&line[byte_index + current.len_utf8()..]);
        out
    } else if crate::display_width(&line) < cols {
        // 行尾：反显空格（ref `:593`）。
        let mut out = String::with_capacity(line.len() + 16);
        out.push_str(&line[..byte_index]);
        out.push_str(crate::diff::CURSOR_MARKER);
        out.push_str("\x1b[7m \x1b[0m");
        out
    } else {
        // 行**已满宽**（`cols` 列）时，反显空格会把该行顶到 `cols + 1`，`diff_render` 会
        // 整帧报错丢弃（`rendered line N exceeds terminal width`）——真机上表现为
        // "输入满一行后不再刷新"。这里改为把**最后一个字符**反显（总宽不变）。
        let last_start = line
            .char_indices()
            .next_back()
            .map_or(0, |(index, _char)| index);
        let mut out = String::with_capacity(line.len() + 16);
        out.push_str(&line[..last_start]);
        out.push_str(crate::diff::CURSOR_MARKER);
        out.push_str("\x1b[7m");
        out.push_str(&line[last_start..]);
        out.push_str("\x1b[0m");
        out
    };
    lines[target] = with_marker;
    lines
}

#[cfg(test)]
mod marker_tests {
    use super::{render_all, render_all_marker, TuiState};

    fn states() -> Vec<(&'static str, TuiState)> {
        let mut out = Vec::new();
        let mut base = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        base.context_files = vec!["AGENTS.md".to_string()];
        out.push(("empty-input", base.clone()));

        let mut typed = base.clone();
        typed.input = "hello".to_string();
        typed.cursor = 5;
        out.push(("typed", typed));

        let mut mid = base.clone();
        mid.input = "hello".to_string();
        mid.cursor = 2;
        out.push(("cursor-mid", mid));

        out
    }

    /// 去掉软件光标的 SGR 包裹（`\u{1b}[7m` / `\u{1b}[0m`），**保留被反显的字符**
    /// （ref `editor.ts:575-600` 是"用反显版本替换该字符"，不是插入额外字符）。
    /// 行尾反显的是一个**额外空格**，因此比较时两侧都 `trim_end`。
    fn strip_software_cursor(line: &str) -> String {
        line.replace("\u{1b}[7m", "").replace("\u{1b}[0m", "")
    }

    /// W1.2 属性测试：剥掉 marker 后，`render_all_marker` 必须与 `render_all` **逐行一致**（含空行）。
    /// ⇒ 证明 `render_all_marker` 是同一装配的**薄封装**（仅插入 marker，不重复装配）。
    #[test]
    fn marker_removal_recovers_render_all_exactly() {
        // P7-6p(续3)/W4(b)(d)：`render_all_marker` 现在是 **r3 专属装配**（多 `[Context]` 段末 1 空行与
        // 工具条目 1 空行，见 `render_all_with` 的 `r3_extra`），因此不再与 `render_all` 逐行相同。
        // 新的性质：**去掉 marker 与 r3 专属空行后**，两侧的**非空行序列**必须完全一致（内容不变、只多空行）。
        for (name, state) in states() {
            let plain = render_all(&state, 100, 30);
            let marked: Vec<String> = render_all_marker(&state, 100, 30)
                .iter()
                .map(|line| strip_software_cursor(&line.replace(crate::diff::CURSOR_MARKER, "")))
                .collect();
            let plain_ne: Vec<String> = plain
                .iter()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim_end().to_string())
                .collect();
            let marked_ne: Vec<String> = marked
                .iter()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim_end().to_string())
                .collect();
            assert_eq!(
                plain_ne, marked_ne,
                "{name}: r3 专属装配改变了**非空行**内容（应只多空行）"
            );
            assert!(
                marked.len() >= plain.len(),
                "{name}: r3 装配不应少于 render_all 的行数"
            );
        }
    }

    /// marker 只在 r3 用的函数里出现；`render_all` 的输出永不含 marker。
    #[test]
    fn render_all_never_contains_marker() {
        for (name, state) in states() {
            for line in render_all(&state, 100, 30) {
                assert!(
                    !line.contains(crate::diff::CURSOR_MARKER),
                    "{name}: render_all 含 marker"
                );
            }
        }
    }
}

#[cfg(test)]
mod t05_oracle_probe {
    use super::{render_all, render_all_marker, TranscriptItem, TuiState};

    /// W2 探针：重建 T05 末帧状态，打印 `render_all` 的行数与末尾 8 行，
    /// 供与 golden 的未折叠尾部（`P7-6p-tail-structure.md`）对照。
    #[test]
    fn t05_state_tail_probe() {
        let mut s = TuiState::new("0.0.0", "work", "1.1%/128k (auto)");
        s.context_files = vec!["AGENTS.md".to_string()];
        s.transcript = vec![
            TranscriptItem::User("say hi".to_string()),
            TranscriptItem::Assistant(
                "# 标题\n\n- 甲\n- 乙\n\n行内 `code` 与结尾。\n\n```\nfn main() {}\n```"
                    .to_string(),
            ),
        ];
        for (name, lines) in [
            ("render_all", render_all(&s, 100, 30)),
            ("render_all_marker", render_all_marker(&s, 100, 30)),
        ] {
            let tail: Vec<String> = lines.iter().rev().take(8).rev().cloned().collect();
            println!("{name}: 行数={} 末尾8行={:?}", lines.len(), tail);
            if name == "render_all_marker" {
                // W2：把 r3 实际喂给 diff_render 的行数组以 JSON 打到 stdout，供临时目录的 TS oracle 使用。
                let body = lines
                    .iter()
                    .map(|line| format!("{:?}", line))
                    .collect::<Vec<_>>()
                    .join(",");
                println!("LINES_JSON={{\"width\":100,\"height\":30,\"lines\":[{body}]}}");
            }
        }
    }
}

#[cfg(test)]
mod t05_frame_export {
    use super::{apply, render_all_marker, TuiAction, TuiEvent, TuiState};

    /// W2：按 r3 的实际驱动顺序（逐字符输入 → Enter → 助手文本）导出**每帧行数组**，
    /// 以 `FRAME_JSON=...` 打到 stdout，供临时目录的 TS oracle 逐帧比对。
    #[test]
    fn export_t05_frames() {
        let mut s = TuiState::new("0.0.0", "work", "1.1%/128k (auto)");
        s.context_files = vec!["AGENTS.md".to_string()];
        let mut frames: Vec<Vec<String>> = Vec::new();
        frames.push(render_all_marker(&s, 100, 30));
        for ch in "say hi".chars() {
            let _ = apply(&mut s, TuiEvent::Char(ch));
            frames.push(render_all_marker(&s, 100, 30));
        }
        if let TuiAction::Submit(_) = apply(&mut s, TuiEvent::Enter) {
            frames.push(render_all_marker(&s, 100, 30));
        }
        s.transcript.push(super::TranscriptItem::Assistant(
            "# 标题\n\n- 甲\n- 乙\n\n行内 `code` 与结尾。\n\n```\nfn main() {}\n```".to_string(),
        ));
        frames.push(render_all_marker(&s, 100, 30));
        let body = frames
            .iter()
            .map(|lines| {
                format!(
                    "[{}]",
                    lines
                        .iter()
                        .map(|line| format!("{:?}", line))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!("FRAME_JSON={{\"width\":100,\"height\":30,\"frames\":[{body}]}}");
        println!("FRAME_COUNT={}", frames.len());
    }
}

#[cfg(test)]
mod cgroup_probe {
    use super::{render_all, render_all_marker, TuiState};

    /// W4(b) 探针：C 组（聊天区为空）末帧的行结构；逐行用 `LINE<n>=<内容>` 输出，便于与 golden 并列比较。
    #[test]
    fn cgroup_tail_probe() {
        let mut s = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        s.context_files = vec!["AGENTS.md".to_string()];
        for (name, lines) in [
            ("render_all", render_all(&s, 100, 30)),
            ("r3", render_all_marker(&s, 100, 30)),
        ] {
            println!("PROBE={name} 行数={}", lines.len());
            for (n, line) in lines.iter().enumerate() {
                println!("{name}LINE{n}={line}");
            }
        }
    }
}

#[cfg(test)]
/// P7-6r2/W3：`/hotkeys` 动态渲染的单测（夹具驱动）。
mod hotkeys_width_tests {
    use super::{hotkeys_panel_for_width, hotkeys_widths, HOTKEYS_LINES};

    #[test]
    fn cols_100_matches_legacy_constant_byte_for_byte() {
        let dynamic = hotkeys_panel_for_width(100);
        assert_eq!(dynamic.len(), HOTKEYS_LINES.len(), "行数不同");
        for (index, (a, b)) in dynamic.iter().zip(HOTKEYS_LINES.iter()).enumerate() {
            assert_eq!(a, b, "第 {index} 行不同");
        }
    }

    #[test]
    fn every_fixture_width_returns_its_own_tier() {
        for (width, expected) in hotkeys_widths::HOTKEYS_PANEL_BY_WIDTH {
            let actual = hotkeys_panel_for_width(*width);
            assert_eq!(actual.len(), expected.len(), "cols={width} 行数不同");
            for (index, (a, b)) in actual.iter().zip(expected.iter()).enumerate() {
                assert_eq!(a, b, "cols={width} 第 {index} 行不同");
            }
        }
    }

    #[test]
    fn uncovered_widths_fall_back_to_lower_tier_not_to_100() {
        // 90 ⇒ 80 档（不大于 90 的最大档），而不是 100 档
        assert_eq!(hotkeys_panel_for_width(90), hotkeys_panel_for_width(80));
        assert_ne!(hotkeys_panel_for_width(90), hotkeys_panel_for_width(100));
        // 50 ⇒ 60 档（低于最小档时用最小档）
        assert_eq!(hotkeys_panel_for_width(50), hotkeys_panel_for_width(60));
        // 200 ⇒ 120 档（最大档）
        assert_eq!(hotkeys_panel_for_width(200), hotkeys_panel_for_width(120));
    }
}

#[cfg(test)]
mod width_probe {
    use super::{render_all_marker, PanelPlacement, TuiState};
    use unicode_width::UnicodeWidthStr;

    /// W3.1：三面板在 cols=60/80/100/120 下的行宽统计（最大宽度、被 clamp 的行数）。
    #[test]
    fn panel_widths_across_cols() {
        let panels: [(&str, &[&str]); 3] = [
            ("HOTKEYS", super::HOTKEYS_LINES),
            ("SETTINGS", super::SETTINGS_LINES),
            ("MODEL", super::MODEL_LINES),
        ];
        for (name, lines) in panels {
            let max_w = lines.iter().map(|l| (*l).width()).max().unwrap_or(0);
            for cols in [60usize, 80, 100, 120] {
                let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
                state.context_files = vec!["AGENTS.md".to_string()];
                state.open_panel(
                    PanelPlacement::BelowEditor,
                    lines.iter().map(|l| (*l).to_string()).collect(),
                );
                let out = render_all_marker(&state, cols, 30);
                let clamped = lines.iter().filter(|l| (*l).width() > cols).count();
                println!(
                    "{name} cols={cols}: 常量行={} 最大宽={max_w} 被clamp={clamped} 输出行={}",
                    lines.len(),
                    out.len()
                );
            }
        }
    }
}

#[cfg(test)]
mod p76al_edit_tests {
    use super::{apply, TuiAction, TuiEvent, TuiState};

    fn state() -> TuiState {
        TuiState::new("0.0.0", "work", "0.0%/128k (auto)")
    }

    /// D：Ctrl+W / Alt+Backspace 只删**当前行**的一个词，不跨换行（ref `editor.ts:1689-1727`）。
    #[test]
    fn delete_word_backward_does_not_cross_lines() {
        let mut s = state();
        for ch in "line one\nline two".chars() {
            apply(&mut s, TuiEvent::Char(ch));
        }
        assert_eq!(
            apply(&mut s, TuiEvent::DeleteWordBackward),
            TuiAction::Redraw
        );
        assert_eq!(s.input, "line one\nline ", "只应删掉本行的 second word");
        // 再按一次：删掉行尾空格后的 "line"
        apply(&mut s, TuiEvent::DeleteWordBackward);
        assert_eq!(s.input, "line one\n");
        // 行首再按：只删换行（与上一行合并），不跨行删词
        apply(&mut s, TuiEvent::DeleteWordBackward);
        assert_eq!(s.input, "line one");
    }
}

#[cfg(test)]
mod p76ao_thinking_tests {
    use super::{apply, render_all, TranscriptItem, TuiAction, TuiEvent, TuiState};

    /// T：thinking 增量进入会话区并**被渲染**（ref `assistant-message.ts:118-150` 默认显示 thinking）。
    #[test]
    fn thinking_delta_is_rendered() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        assert_eq!(
            apply(&mut state, TuiEvent::ThinkingDelta("pondering".to_string())),
            TuiAction::Redraw
        );
        apply(&mut state, TuiEvent::ThinkingDelta(" deeply".to_string()));
        // 连续增量合并到同一条 Thinking 条目
        assert_eq!(state.transcript.len(), 1);
        matches!(&state.transcript[0], TranscriptItem::Thinking(text) if text == "pondering deeply");
        // 正常助手文本紧随其后应新建 Assistant 条目（不覆盖 thinking）
        apply(&mut state, TuiEvent::AssistantDelta("answer".to_string()));
        assert_eq!(state.transcript.len(), 2);
        let lines = render_all(&state, 100, 30).join("\n");
        assert!(
            lines.contains("pondering deeply"),
            "thinking 文本应被渲染：{lines}"
        );
        assert!(lines.contains("answer"), "助手文本应被渲染：{lines}");
    }
}

#[cfg(test)]
mod soft_cursor_tests {
    use super::{render_all_marker, TuiState};

    /// ref `editor.ts:575-600`：`showHardwareCursor=false`（默认）时编辑器画**反显软件光标**。
    /// 行中 ⇒ 反显该字符；行尾 ⇒ 反显一个空格。
    #[test]
    fn software_cursor_is_rendered() {
        let mut mid = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        mid.input = "hel".to_string();
        mid.cursor = 1;
        let lines = render_all_marker(&mid, 40, 12);
        let row = lines
            .iter()
            .find(|line| line.contains("\u{1b}_pi:c\u{7}"))
            .expect("应有光标标记行");
        assert!(
            row.contains("\u{1b}[7me\u{1b}[0m"),
            "行中应反显字符：{row:?}"
        );

        let mut end = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        end.input = "hel".to_string();
        end.cursor = 3;
        let lines = render_all_marker(&end, 40, 12);
        let row = lines
            .iter()
            .find(|line| line.contains("\u{1b}_pi:c\u{7}"))
            .expect("应有光标标记行");
        assert!(
            row.contains("\u{1b}[7m \u{1b}[0m"),
            "行尾应反显空格：{row:?}"
        );
    }
}

#[cfg(test)]
mod p78a_completion_after_edit_tests {
    use super::{apply, TuiEvent, TuiState};

    fn typed(text: &str) -> TuiState {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        for ch in text.chars() {
            let _ = apply(&mut state, TuiEvent::Char(ch));
        }
        state
    }

    /// 真机反馈：`/tree` 打全后菜单关闭（精确匹配），**退格删掉字符后菜单必须重新出现**。
    /// ref `editor.ts:1444`（backspace 后重新触发 autocomplete）。
    #[test]
    fn backspace_reopens_completion() {
        let mut state = typed("/tree");
        // TS 在恰好等于命令名时菜单仍可见（T68 帧 after-tree），这里也保持可见。
        assert!(state.completion_open, "打全命令名后菜单仍可见（TS 同款）");
        let _ = apply(&mut state, TuiEvent::Backspace); // "/tre"
        assert!(state.completion_open, "退格后菜单应重新出现");
        let _ = apply(&mut state, TuiEvent::Backspace); // "/tr"
        assert!(state.completion_open, "继续退格仍在补全范围");
        for _ in 0..2 {
            let _ = apply(&mut state, TuiEvent::Backspace); // "/"
        }
        assert_eq!(state.input, "/");
        assert!(state.completion_open, "只剩 `/` 时菜单应打开");
        let _ = apply(&mut state, TuiEvent::Backspace); // ""
        assert!(!state.completion_open, "输入为空时菜单应关闭");
    }

    /// 反证用例：`/` 退格回空输入时菜单必须**关掉**（不清开关就会留下过期菜单）。
    #[test]
    fn backspace_to_empty_closes_completion() {
        let mut state = typed("/");
        assert!(state.completion_open, "`/` 应列出全部命令");
        let _ = apply(&mut state, TuiEvent::Backspace);
        assert_eq!(state.input, "");
        assert!(!state.completion_open, "空输入不得残留补全菜单");
    }

    /// 删除光标前的字符（Delete）后同理（ref `editor.ts:1809`）。
    #[test]
    fn forward_delete_keeps_completion_consistent() {
        let mut state = typed("/set");
        assert!(state.completion_open, "`/set` 是前缀匹配，菜单应打开");
        // 把光标移到行首再 Delete，删掉 `/` ⇒ 不再是斜杠命令
        let _ = apply(&mut state, TuiEvent::LineStart);
        let _ = apply(&mut state, TuiEvent::Delete);
        assert_eq!(state.input, "set");
        assert!(!state.completion_open, "删掉 `/` 后菜单应关闭");
    }
}

#[cfg(test)]
mod p78a_width_overflow_tests {
    use super::{apply, render_all_marker, TuiEvent, TuiState};

    fn typed(text: &str) -> TuiState {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        for ch in text.chars() {
            let _ = apply(&mut state, TuiEvent::Char(ch));
        }
        state
    }

    /// 真机现象：CJK 输入把某一行顶到 `cols + 1` 列，`diff_render` 报
    /// `rendered line N exceeds terminal width` 并**整帧丢弃**（表现为卡顿/不刷新）。
    /// 这里断言：任何输入、任何宽度下，渲染出的每一行显示宽度都 ≤ `cols`。
    #[test]
    fn every_line_fits_the_terminal_width() {
        let long_ascii = "a".repeat(120);
        let long_cjk = "中".repeat(60);
        let inputs = [
            "abc 中文xyz",
            "中文",
            "a中",
            "中文中文中文",
            "👨‍👩‍👧 中",
            long_ascii.as_str(),
            long_cjk.as_str(),
            "ab 中文ab 中文ab 中文ab 中文ab 中文ab 中文ab 中文ab 中文ab 中文ab 中文ab 中文ab 中文",
        ];
        for cols in [20_usize, 40, 80, 100, 200] {
            for text in inputs {
                let state = typed(text);
                for (index, line) in render_all_marker(&state, cols, 30).iter().enumerate() {
                    let visible = strip_marker(line);
                    let width = super::display_width(&visible);
                    assert!(
                        width <= cols,
                        "cols={cols} text={text:?} 第 {index} 行宽 {width} > {cols}：{visible:?}"
                    );
                }
            }
        }
    }

    /// 去掉反显光标标记与零宽光标标记（OSC），只留可见字符。
    fn strip_marker(line: &str) -> String {
        let mut out = String::new();
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\u{1b}' {
                match chars.peek() {
                    Some('[') => {
                        // CSI：跳到终止字节
                        for next in chars.by_ref() {
                            if ('@'..='~').contains(&next) && next != '[' {
                                break;
                            }
                        }
                    }
                    Some('_') => {
                        // OSC（CURSOR_MARKER）：跳到 BEL
                        for next in chars.by_ref() {
                            if next == '\u{7}' {
                                break;
                            }
                        }
                    }
                    _other => {}
                }
                continue;
            }
            out.push(ch);
        }
        out
    }
}

#[cfg(test)]
mod p78c_paste_tests {
    use super::{apply, TuiAction, TuiEvent, TuiState};

    fn state() -> TuiState {
        TuiState::new("0.0.0", "work", "0.0%/128k (auto)")
    }

    fn paste(state: &mut TuiState, text: &str) {
        let _ = apply(state, TuiEvent::Paste(text.to_string()));
    }

    /// ref `editor.ts:1301-1313`：**超过 10 行**或**超过 1000 字符**才折叠成占位符，
    /// 行数优先（`pastedLines.length > 10 ? "+N lines" : "N chars"`）。
    #[test]
    fn large_paste_is_folded_into_marker() {
        let mut s = state();
        let twelve_lines = "a\n".repeat(11) + "a";
        paste(&mut s, &twelve_lines);
        assert_eq!(s.input, "[paste #1 +12 lines]");

        let mut s = state();
        let long_single_line = "x".repeat(1001);
        paste(&mut s, &long_single_line);
        assert_eq!(s.input, "[paste #1 1001 chars]");
    }

    /// 边界：恰好 10 行、恰好 1000 字符**不折叠**（`>` 不是 `>=`）。
    #[test]
    fn boundary_pastes_are_not_folded() {
        let mut s = state();
        let ten_lines = "a\n".repeat(9) + "a";
        paste(&mut s, &ten_lines);
        assert_eq!(s.input, ten_lines);

        let mut s = state();
        let exactly = "y".repeat(1000);
        paste(&mut s, &exactly);
        assert_eq!(s.input, exactly);
    }

    /// 提交时必须展开回原文（ref `getExpandedText`）。
    #[test]
    fn submit_expands_markers() {
        let mut s = state();
        let text = "line\n".repeat(20);
        paste(&mut s, &text);
        assert!(s.input.starts_with("[paste #1"));
        match apply(&mut s, TuiEvent::Enter) {
            TuiAction::Submit(submitted) => {
                assert!(
                    submitted.contains("line\nline"),
                    "提交应展开原文：{submitted:?}"
                );
                assert!(!submitted.contains("[paste #"), "提交不应带占位符");
            }
            other => panic!("应提交，实际 {other:?}"),
        }
    }

    /// 退格键把占位符当作**一个整体**删除（ref `segmentWithMarkers` 的原子段）。
    #[test]
    fn backspace_removes_whole_marker() {
        let mut s = state();
        let text = "x\n".repeat(15);
        paste(&mut s, &text);
        // ref 用 `filteredText.split("\n").length`：末尾换行算一行 ⇒ 16。
        assert_eq!(s.input, "[paste #1 +16 lines]");
        let _ = apply(&mut s, TuiEvent::Backspace);
        assert_eq!(s.input, "", "一次退格应删掉整个占位符");
    }

    /// 多次粘贴编号递增；清空输入不重置编号（只有 `setText` 才重置）。
    #[test]
    fn paste_ids_increment() {
        let mut s = state();
        paste(&mut s, &"a\n".repeat(12));
        paste(&mut s, &"b\n".repeat(12));
        assert_eq!(s.input, "[paste #1 +13 lines][paste #2 +13 lines]");
    }

    /// 控制字符被过滤（保留 `\n`），照 ref `editor.ts:1280-1284`。
    #[test]
    fn control_chars_are_filtered() {
        let mut s = state();
        paste(&mut s, "a\u{1}b\tc");
        assert_eq!(s.input, "abc");
    }
}

#[cfg(test)]
mod p8a_spinner_tests {
    use super::{apply, render, TuiEvent, TuiState, SPINNER_FRAMES, SPINNER_INTERVAL};

    /// ref `loader.ts:13-14/79-86`：10 帧、80 ms 一换，循环播放。
    #[test]
    fn spinner_frames_cycle_and_are_rendered() {
        assert_eq!(SPINNER_FRAMES.len(), 10);
        assert_eq!(SPINNER_INTERVAL.as_millis(), 80);
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        let _ = apply(&mut state, TuiEvent::Working(true));
        let first = render(&state, 100, 30)
            .into_iter()
            .find(|line| line.contains("Working") && line.starts_with("──"))
            .expect("应有 Working 行");
        assert!(
            first.contains(SPINNER_FRAMES[0]),
            "首帧应是 {SPINNER_FRAMES:?}：{first}"
        );
        state.tick_spinner();
        let second = render(&state, 100, 30)
            .into_iter()
            .find(|line| line.contains("Working") && line.starts_with("──"))
            .expect("应有 Working 行");
        assert!(
            second.contains(SPINNER_FRAMES[1]),
            "推进一帧后应是 {}：{second}",
            SPINNER_FRAMES[1]
        );
        // 走满一圈回到起点（循环而不是停住）。
        for _ in 0..9 {
            state.tick_spinner();
        }
        assert_eq!(state.spinner_frame, 0);
    }
}

#[cfg(test)]
mod p8b_thinking_tests {
    use super::{apply, TranscriptItem, TuiEvent, TuiState};

    /// ref `interactive-mode.ts:4367-4378`：`shift+tab` 循环档位并显示 `Thinking level: X`；
    /// 非推理模型（只有 `off`）显示 `Current model does not support thinking`。
    #[test]
    fn shift_tab_cycles_thinking_levels() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        state.available_thinking_levels = ["off", "low", "medium", "high"]
            .iter()
            .map(|level| (*level).to_string())
            .collect();
        state.thinking_level = "medium".to_string();
        let _ = apply(&mut state, TuiEvent::BackTab);
        assert_eq!(state.thinking_level, "high");
        let _ = apply(&mut state, TuiEvent::BackTab);
        assert_eq!(state.thinking_level, "off", "到末尾要回绕到第一个");
        let statuses: Vec<String> = state
            .transcript
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Status(text) => Some(text.clone()),
                _other => None,
            })
            .collect();
        // ref `showStatus`：连续状态行**原地替换**——按两次只应留下**一行**，内容是最新一次。
        assert_eq!(statuses.len(), 1, "状态行不应累积：{statuses:?}");
        assert_eq!(statuses[0], "Thinking level: off");
    }

    /// 非推理模型：即使按了 `shift+tab` 也不会切档，只提示不支持。
    #[test]
    fn non_reasoning_model_shows_notice() {
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        let _ = apply(&mut state, TuiEvent::BackTab);
        assert_eq!(state.thinking_level, "off");
        assert!(state.transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::Status(text) if text == "Current model does not support thinking"
        )));
    }
}
