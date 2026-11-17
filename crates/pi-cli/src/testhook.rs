//! F53/T2：**非 ref 的测试设施**——把"按键序列 + 每步屏幕/状态快照"变成可自动化的事实，
//! 让真机类缺陷（页脚分支名、thinking 档位、ctrl+o/ctrl+t 折叠）能由 agent 在
//! 真实配置副本上复现与验收，而不依赖人工盯屏。
//!
//! 只由环境变量启用；**全部未设置时零行为差异**（不建文件、不注入按键、不额外格式化）：
//!
//! - `PI_TEST_KEYS`：首帧稳定后按序注入的按键，逗号分隔的键名（`ctrl+o`、`ctrl+t`…），
//!   名字走 [`crate::tui::map_key_sequence`] 同一张表（与真实按键同一路径）。
//! - `PI_TEST_DUMP=<dir>`：每步（初态 + 每个按键之后）写
//!   `step-N.txt`（去 ANSI 的可见屏幕文本）与 `step-N.json`（状态快照）。
//! - `PI_TEST_TAIL=N`：退出时把最终屏幕最后 N 行写进 `tail.txt`。
//!
//! 步进的"静止"判定用**重绘计数在空闲轮询里不再增长**（连续
//! [`QUIET_TICKS`] 次空闲轮询没有新帧），不用固定延迟——固定延迟会让慢机器上
//! 的快照落在半帧上，也会让快机器白等。
//!
//! 不属于 ref 行为，已记入 `LIMITATIONS.md`。

use std::path::{Path, PathBuf};

/// 连续多少次"空闲轮询且无新帧"算静止。
///
/// 一轮空闲轮询是主循环的 `recv_timeout` 超时（无事件 200 ms）。两次用于消化
/// 后台线程刚发出、还在路上的扩展渲染结果（`tool_row_pending`/`ui_render_pending`
/// 非空时本计数不推进）。
const QUIET_TICKS: u32 = 2;

/// 测试钩子配置。`from_env()` 返回 `None` 即"钩子未启用"。
#[derive(Debug, Clone)]
pub struct TestHook {
    keys: std::collections::VecDeque<String>,
    /// 序列跑完的标记：**只写最后一次快照**。
    ///
    /// 为什么需要：`note_idle` 每 `QUIET_TICKS` 个空闲轮询就返回一次 true，
    /// `advance()` 只把计数清零、不会让"空闲"这件事停止发生。按键用完后仍会每
    /// `QUIET_TICKS` 轮写一份 `step-N.txt` + `marks.jsonl`（实测 116 MB 会话 80 份，
    /// real-parity 因此判 different）。这里用一个显式开关把"已收尾"钉住。
    done: bool,
    dump_dir: Option<PathBuf>,
    tail: usize,
    step: usize,
    quiet_ticks: u32,
    injected: usize,
}

impl TestHook {
    /// 从环境变量读取。三个变量**全部未设置**时返回 `None`。
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let keys_env = std::env::var("PI_TEST_KEYS").ok();
        let dump_env = std::env::var("PI_TEST_DUMP").ok();
        let tail_env = std::env::var("PI_TEST_TAIL").ok();
        if keys_env.is_none() && dump_env.is_none() && tail_env.is_none() {
            return None;
        }
        // `type:<文本>` 展开成逐字符注入（F54/Z3：长文本/CJK/超宽串/粘贴块）。
        let keys = Self::expand_keys(&keys_env.unwrap_or_default());
        let dump_dir = dump_env
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        if let Some(dir) = dump_dir.as_deref() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tail = tail_env
            .as_deref()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        Some(Self {
            keys,
            done: false,
            dump_dir,
            tail,
            step: 0,
            quiet_ticks: 0,
            injected: 0,
        })
    }

    /// 有帧画出 ⇒ 还没静止。
    pub fn note_frame(&mut self) {
        self.quiet_ticks = 0;
    }

    /// 一轮**空闲**轮询（没有任何事件、也没有等待中的渲染）。
    ///
    /// 返回 `true` 表示"已经静止，可以进入下一步"。
    pub fn note_idle(&mut self) -> bool {
        if self.quiet_ticks < QUIET_TICKS {
            self.quiet_ticks += 1;
        }
        self.quiet_ticks >= QUIET_TICKS
    }

    /// 下一次要注入的按键（键名）。`None` = 序列跑完。
    pub fn next_key(&mut self) -> Option<String> {
        let key = self.keys.pop_front()?;
        self.injected += 1;
        Some(key)
    }

    /// 把 `prefix:<文本>` 形式的键展开成逐字符追加的**文本**（F54/Z3）。
    ///
    /// 为什么用 `type:` 前缀：文本里可能有中文、可能有几十个字符，直接写进
    /// `PI_TEST_KEYS`（逗号分隔）既不可读也不安全。`type:` 之后的整段按字符逐个注入，
    /// 每个字符之间不额外等静止（与真人连打一致，也不依赖固定延迟）。
    pub fn expand_keys(raw: &str) -> std::collections::VecDeque<String> {
        let mut out = std::collections::VecDeque::new();
        for item in raw
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            match item.strip_prefix("type:") {
                Some(text) => {
                    for ch in text.chars() {
                        out.push_back(format!("char:{ch}"));
                    }
                }
                None => out.push_back(item.to_string()),
            }
        }
        out
    }

    pub fn dump_dir(&self) -> Option<&Path> {
        self.dump_dir.as_deref()
    }

    pub fn tail(&self) -> usize {
        self.tail
    }

    /// 本步编号（初态为 0）。
    pub fn step(&self) -> usize {
        self.step
    }

    pub fn advance(&mut self) {
        self.step += 1;
        self.quiet_ticks = 0;
    }

    /// 标记"按键序列已跑完"：之后不再写任何快照。
    pub fn finish(&mut self) {
        self.done = true;
    }

    /// 是否已经收尾（`done` 之后不再产出快照）。
    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn injected(&self) -> usize {
        self.injected
    }

    /// 写 `step-N.txt` 与 `step-N.json`；`dump_dir` 未设置时**什么都不做**。
    pub fn write_step(
        &self,
        screen: &[String],
        state: &pi_tui::TuiState,
        n: u64,
        cols: usize,
        rows: usize,
    ) {
        let Some(dir) = self.dump_dir.as_deref() else {
            return;
        };
        let text = screen
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(
            dir.join(format!("step-{}.txt", self.step)),
            format!("{text}\n"),
        );
        // 记录本步对应的**已写字节偏移**：`run-real.sh` 用它像 TS 侧一样从 raw.bin
        // 还原屏幕（`marks-to-steps.mjs`），两侧走同一条渲染路径。
        // 自己重算屏幕与终端实际显示会差 2 行（增量渲染的视口与全帧尾部不总是重合）。
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("marks.jsonl"))
        {
            use std::io::Write as _;
            let offset = WRITTEN_BYTES.load(std::sync::atomic::Ordering::SeqCst);
            let _ = writeln!(
                file,
                "{{\"name\": \"step{}\", \"offset\": {offset}}}",
                self.step
            );
        }
        let _ = std::fs::write(
            dir.join(format!("step-{}.json", self.step)),
            step_json(state, self.step, n, cols, rows).to_string(),
        );
    }

    /// 退出时写 `tail.txt`（最后 `PI_TEST_TAIL` 行）。
    pub fn write_tail(&self, screen: &[String]) {
        let Some(dir) = self.dump_dir.as_deref() else {
            return;
        };
        if self.tail == 0 {
            return;
        }
        let skip = screen.len().saturating_sub(self.tail);
        let text = screen
            .iter()
            .skip(skip)
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(dir.join("tail.txt"), format!("{text}\n"));
    }
}

/// 键名 → 原始字节序列。
///
/// 为什么不让 [`crate::tui::map_key_sequence`] 直接吃键名：那个函数按**原始序列**解析
/// （`parse_key("ctrl+o")` 会把首字符 `c` 当成普通字符 ⇒ `Some(Char('c'))`，实测）。
/// 这里先把名字翻成字节，再走同一条 `bytes → TuiEvent` 路径，注入的键与真实按键等价。
#[must_use]
pub fn key_bytes_for_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "ctrl+o" => "\u{0f}",
        "ctrl+t" => "\u{14}",
        "ctrl+d" => "\u{04}",
        "ctrl+p" => "\u{10}",
        "ctrl+n" => "\u{0e}",
        "ctrl+s" => "\u{13}",
        "ctrl+l" => "\u{0c}",
        "ctrl+c" => "\u{03}",
        "ctrl+k" => "\u{0b}",
        "ctrl+z" => "\u{1a}",
        "ctrl+r" => "\u{12}",
        "ctrl+x" => "\u{18}",
        "shift+tab" => "\u{1b}[Z",
        "escape" => "\u{1b}",
        // F61/30：`app.message.followUp` = `alt+enter`（ref `core/keybindings.ts:133-136`）。
        "alt+enter" => "\u{1b}\r",
        // `app.message.dequeue` = `alt+up`（`:137-140`）；xterm 的 modifyOtherKeys 编码。
        "alt+up" => "\u{1b}[1;3A",
        // F61/32：`app.session.deleteNoninvasive` = `ctrl+backspace`；kitty 编码。
        "ctrl+backspace" => "\u{1b}[127;5u",
        // F61/35：`app.clipboard.pasteImage` = `ctrl+v`。
        "ctrl+v" => "\u{16}",
        "enter" => "\r",
        // F61/19：面板里空格是"切换启用"（ref `scoped-models-selector.ts:299-310`）。
        // 缺这一条时 `space` 会被当成 5 个字母的**文本**注入（实测：面板选中项不变）。
        "space" => " ",
        "tab" => "\t",
        "backspace" => "\u{7f}",
        // F54/Z3：方向键与 Home/End（编辑器折行后的行内/跨行移动）。
        "left" => "\u{1b}[D",
        "right" => "\u{1b}[C",
        "up" => "\u{1b}[A",
        "down" => "\u{1b}[B",
        "home" => "\u{1b}[H",
        "end" => "\u{1b}[F",
        "delete" => "\u{1b}[3~",
        _other => return None,
    })
}

/// 每步的状态快照（调试与断言用；字段名即契约，`real-parity.sh`/`self-verify.sh` 依赖）。
fn step_json(
    state: &pi_tui::TuiState,
    step: usize,
    draw_count: u64,
    cols: usize,
    rows: usize,
) -> serde_json::Value {
    let branch = pi_tui::gitinfo::git_branch(std::path::Path::new(&state.cwd));
    let tools: Vec<serde_json::Value> = state
        .transcript
        .iter()
        .filter_map(|item| match item {
            pi_tui::TranscriptItem::ToolCall {
                id,
                name,
                collapsed_row,
                ..
            } => Some(serde_json::json!({
                "id": id,
                "name": name,
                "collapsed_row": collapsed_row.is_some(),
                "collapsed_lines": collapsed_row.as_ref().map_or(0, Vec::len),
            })),
            _other => None,
        })
        .collect();
    let items = item_spans(state, cols);
    // 帧内容行数（未补齐、未裁剪）：与 `rows` 比较可判断这一帧是否触发上滚。
    let frame_lines = pi_tui::render_all(state, cols, rows).len();
    serde_json::json!({
        "items": items,
        "frame_lines": frame_lines,
        "step": step,
        "draw_count": draw_count,
        "cols": cols,
        "rows": rows,
        "tools_expanded": state.tools_expanded,
        "hide_thinking": state.hide_thinking_block,
        "thinking_level": state.thinking_level,
        "available_thinking_levels": state.available_thinking_levels,
        "footer": {
            "cwd_line": pi_tui::gitinfo::footer_cwd_line(&state.cwd, state.home.as_deref()),
            "branch": branch,
            "status": state.status,
            "model": state.model,
            "model_provider": state.model_provider,
            "extension_footer": state.footer_override.is_some(),
        },
        "tools": tools,
        "ext": ext_counters(),
    })
}

/// 每个转录条目占用的行数（F53 逐条目对拍用）。
///
/// 用"前缀渲染差分"算：把转录截到第 `i` 条渲染一次，取**聊天区**行数，与第 `i-1` 次相减。
/// 聊天区行数 = 内容末行下标 + 1 − 编辑器框与状态区的固定行数（`render_all` 的布局是
/// 头部 → 聊天区 → 编辑器框 3 行 → 状态区 2 行）。`rows` 取足够大以免上滚干扰。
/// 只在测试钩子写 dump 时调用。
fn item_spans(state: &pi_tui::TuiState, cols: usize) -> Vec<serde_json::Value> {
    // 只要超过最长聊天区即可（真实会话约 2100 行）；过大只是白算填充行。
    const TALL: usize = 8192;
    /// 编辑器框 3 行 + 状态区 2 行（`render_all` 的固定尾部）。
    const FOOTER_LINES: usize = 5;
    let chat_len = |frame: &[String]| -> usize {
        let end = frame
            .iter()
            .rposition(|line| !line.trim().is_empty())
            .map_or(0, |last| last + 1);
        end.saturating_sub(FOOTER_LINES)
    };
    let mut empty = state.clone();
    empty.transcript.clear();
    let header_len = chat_len(&pi_tui::render_all(&empty, cols, TALL));
    let mut out = Vec::new();
    let mut previous = header_len;
    for index in 1..=state.transcript.len() {
        let mut prefix = state.clone();
        prefix.transcript.truncate(index);
        let frame = pi_tui::render_all(&prefix, cols, TALL);
        let len = chat_len(&frame);
        let first = frame
            .iter()
            .take(len)
            .skip(previous)
            .find(|line| !line.trim().is_empty())
            .cloned()
            .unwrap_or_default();
        out.push(serde_json::json!({
            "i": index - 1,
            "kind": item_kind(&prefix.transcript[index - 1]),
            "lines": len.saturating_sub(previous),
            "first": first,
        }));
        previous = len;
    }
    out
}

/// 条目类型名（只用于 dump 对比，不参与渲染）。
fn item_kind(item: &pi_tui::TranscriptItem) -> &'static str {
    use pi_tui::TranscriptItem as Item;
    match item {
        Item::User(_) => "user",
        Item::Assistant(_) => "assistant",
        Item::Thinking { .. } => "thinking",
        Item::ToolCall { .. } => "tool",
        Item::BashExecution { .. } => "bash",
        Item::Notice(_) => "notice",
        Item::Warning(_) => "warning",
        Item::Status(_) => "status",
        Item::SkillInvocation { .. } => "skill",
        Item::BranchSummary { .. } => "branch",
        Item::CompactionSummary { .. } => "compaction",
    }
}

/// 扩展渲染请求的计数（`ext.render.tool` / `ext.render.thinking`）。
///
/// 只在钩子启用时被读取；未启用时不写文件也不读取，计数器本身是两条 `AtomicU64`
/// （写入点是一次 `fetch_add`，与"是否有钩子"无关，但开销可忽略且不改变任何输出）。
fn ext_counters() -> serde_json::Value {
    serde_json::json!({
        "render_tool_requests": crate::ext_tools::test_render_tool_requests(),
        "render_tool_rows": crate::ext_tools::test_render_tool_rows(),
        "thinking_requests": crate::ext_tools::test_thinking_requests(),
        "thinking_labels": crate::ext_tools::test_thinking_labels(),
    })
}

/// 去掉 ANSI 转义（CSI / OSC），只留可见字符。
///
/// 与 `pi_tui` 里测试用的同名函数同一语义：`dump` 的文本要能直接与 TS 侧
/// 用 `renderScreen` 还原的屏幕文本逐行比较。
#[must_use]
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                for next in chars.by_ref() {
                    if next == '\x07' {
                        break;
                    }
                }
            }
            Some('_') => {
                // APC（`\x1b_pi:c\x07`）：OSC 之外的另一种"私有序列"。
                chars.next();
                for next in chars.by_ref() {
                    if next == '\x07' {
                        break;
                    }
                }
            }
            _other => {}
        }
    }
    out
}

/// 最近一次**真正写给终端**的帧（`PI_TEST_DUMP` 用；未启用钩子时恒为 `None`）。
static LAST_FRAME: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

/// 已写给终端的字节总数（`PI_TEST_DUMP` 用）：截图按**字节偏移**从录制流里还原，
/// 与 TS 侧（`marks-to-steps`）走完全相同的口径，避免"重算帧 ≠ 终端屏幕"。
static WRITTEN_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 记录一帧（由 TUI 在写出前调用；只在 `PI_TEST_DUMP` 启用时被调用）。
///
/// 为什么不用 `render_all` 重新算一遍：截图必须等于**终端实际收到的帧**。
/// 重算会在"帧之后状态又变了"时给出不同结果（真实会话 40 行实测：dump 比屏幕多 2 行）。
pub fn dump_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PI_TEST_DUMP").is_some())
}

/// 记录一帧已写出的字节数（由 `draw` 在写成功后调用）。
pub fn record_bytes(len: usize) {
    WRITTEN_BYTES.fetch_add(len, std::sync::atomic::Ordering::SeqCst);
}

pub fn record_frame(lines: &[String]) {
    if let Ok(mut slot) = LAST_FRAME.lock() {
        *slot = Some(lines.to_vec());
    }
}

pub fn visible_screen(state: &pi_tui::TuiState, cols: usize, rows: usize) -> Vec<String> {
    // 与终端实际显示一致的口径：`diff_render` 把**整帧**写给终端、终端自己只留最后
    // `rows` 行（`crates/pi-tui/src/diff.rs` 的 `full_render` 写全部行）。
    // 所以这里取 `render_all` 的**末 `rows` 行**，而不是 `render()`（后者按会话区
    // 容量裁 chat，会额外少 2 行；实测 40 行终端下 dump 与真实屏幕差 2 行）。
    let frame = match LAST_FRAME.lock().ok().and_then(|slot| slot.clone()) {
        Some(lines) if !lines.is_empty() => lines,
        _other => pi_tui::render_all(state, cols, rows),
    };
    let skip = frame.len().saturating_sub(rows);
    let mut visible: Vec<String> = frame.into_iter().skip(skip).collect();
    // 去掉**尾部**空行：与 `compare-tui.mjs` 的 `normalizeScreen` 同口径
    // （否则终端比内容高时，两侧的尾随空行数不同会造成假差异）。
    while visible.last().is_some_and(|line| line.trim().is_empty()) {
        visible.pop();
    }
    visible
}

/// 钩子的"静止"判定需要的事件循环空闲单位（供文档与测试引用）。
#[must_use]
pub fn quiet_ticks() -> u32 {
    QUIET_TICKS
}

/// 超时文本（仅用于测试里断言钩子未启用时零写入）。
#[must_use]
pub fn dump_is_disabled() -> bool {
    TestHook::from_env().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_tui::{TranscriptItem, TuiState};

    /// 环境变量是**进程级**的：下面几个用例都会读写 `PI_TEST_*`，必须串行执行，
    /// 否则 `disabled_without_env` 会撞上别的用例刚设好的变量
    /// （`cargo test --workspace` 并行跑时实测偶发失败）。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn state() -> TuiState {
        let mut st = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
        st.transcript.push(TranscriptItem::User("hi".into()));
        st
    }

    /// 三个环境变量都未设置 ⇒ 钩子完全不启用（不建目录、不注入、不写文件）。
    #[test]
    fn disabled_without_env() {
        let _guard = env_guard();
        for key in ["PI_TEST_KEYS", "PI_TEST_DUMP", "PI_TEST_TAIL"] {
            std::env::remove_var(key);
        }
        assert!(TestHook::from_env().is_none());
        assert!(dump_is_disabled());
    }

    /// 启用钩子**不改变渲染输出**：同一状态下开/关钩子渲染逐字节相同。
    #[test]
    fn hook_does_not_change_rendering() {
        let _guard = env_guard();
        let st = state();
        let plain = pi_tui::render_all(&st, 80, 30);
        std::env::set_var("PI_TEST_KEYS", "ctrl+o,ctrl+t");
        std::env::set_var("PI_TEST_TAIL", "3");
        let with_hook = pi_tui::render_all(&st, 80, 30);
        std::env::remove_var("PI_TEST_KEYS");
        std::env::remove_var("PI_TEST_TAIL");
        assert_eq!(plain, with_hook);
    }

    /// 步进静止判定：连续两次空闲才算静止，中间有帧就清零。
    #[test]
    fn quiet_requires_two_idle_ticks() {
        let _guard = env_guard();
        std::env::set_var("PI_TEST_KEYS", "ctrl+o");
        let mut hook = TestHook::from_env().expect("钩子应启用");
        std::env::remove_var("PI_TEST_KEYS");
        assert!(!hook.note_idle());
        hook.note_frame();
        assert!(!hook.note_idle());
        assert!(hook.note_idle());
    }

    /// `step-N.txt` 去 ANSI、`step-N.json` 带关键字段；`tail.txt` 只留最后 N 行。
    #[test]
    fn writes_step_and_tail_files() {
        let _guard = env_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_var("PI_TEST_KEYS", "");
        std::env::set_var("PI_TEST_DUMP", dir.path().to_str().expect("utf8"));
        std::env::set_var("PI_TEST_TAIL", "2");
        let hook = TestHook::from_env().expect("钩子应启用");
        std::env::remove_var("PI_TEST_KEYS");
        std::env::remove_var("PI_TEST_DUMP");
        std::env::remove_var("PI_TEST_TAIL");
        let st = state();
        let screen = vec!["\u{1b}[31mred\u{1b}[0m".to_string(), "plain".to_string()];
        hook.write_step(&screen, &st, 7, 80, 24);
        let txt = std::fs::read_to_string(dir.path().join("step-0.txt")).expect("读 step txt");
        assert!(txt.contains("red") && !txt.contains('\u{1b}'), "{txt}");
        let json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("step-0.json")).expect("读 json"),
        )
        .expect("json");
        assert_eq!(json["tools_expanded"], serde_json::json!(false));
        assert_eq!(json["footer"]["extension_footer"], serde_json::json!(false));
        assert!(json["ext"]["render_tool_requests"].is_number());
        hook.write_tail(&screen);
        let tail = std::fs::read_to_string(dir.path().join("tail.txt")).expect("读 tail");
        assert_eq!(tail.trim_end(), "red\nplain");
    }

    /// `strip_ansi` 覆盖 CSI、OSC 与 APC（`\x1b_pi:c\x07` 真机实测会出现）。
    #[test]
    fn strips_csi_osc_apc() {
        assert_eq!(strip_ansi("\u{1b}[38;5;1mx\u{1b}[0m"), "x");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}y"), "y");
        assert_eq!(strip_ansi("\u{1b}_pi:c\u{7}z"), "z");
    }

    /// 键名 → 字节 → `TuiEvent` 与真实按键同一路径（`ctrl+o` 曾被解析成 `Char('c')`）。
    #[test]
    fn key_names_map_to_the_same_events_as_keystrokes() {
        use crate::tui::map_key_sequence;
        use pi_tui::TuiEvent;
        for (name, expect) in [
            ("ctrl+o", TuiEvent::CtrlO),
            ("ctrl+t", TuiEvent::CtrlT),
            ("ctrl+d", TuiEvent::CtrlD),
            ("shift+tab", TuiEvent::BackTab),
            ("escape", TuiEvent::Escape),
        ] {
            let bytes = key_bytes_for_name(name).expect("已知键名");
            assert_eq!(map_key_sequence(bytes), Some(expect), "键名 {name}");
        }
        assert!(key_bytes_for_name("no-such-key").is_none());
    }
}
