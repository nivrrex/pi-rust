//! 主屏差分渲染的**纯函数**移植（规格：`ref/pi-0.87.1/packages/tui/src/tui-main-screen.ts:247-618` 的
//! `TuiMainScreen.doRender` 与 `tui.ts:1353-1358` 的 `applyLineResets`）。
//!
//! **本批不接线**：`pi-cli` 的渲染路径与 R1 行为均未改动；`--tui-mode` 仍拒绝；`capacity` 未动。
//!
//! 线程安全：`diff_render` 是纯函数（输入 `&mut DiffState` + 行数组 + 宽高，输出字节串与更新后的状态），
//! 不含全局可变状态、不做 I/O；`DiffState` 为 `Send + Sync`。
use unicode_width::UnicodeWidthStr;

/// 每行末尾的分段复位（`tui.ts:384` `const SEGMENT_RESET = "\x1b[0m\x1b]8;;\x07"`）。
const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";

/// 同步输出区开始/结束（`tui-main-screen.ts:283`、`:317`、`:494`、`:567`）。
const SYNC_START: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";
/// 整屏重绘的清屏序列（`tui-main-screen.ts:283`）。
const CLEAR_ALL: &str = "\x1b[2J\x1b[H\x1b[3J";
/// 光标标记：组件在需要硬件光标时把它放进行内（`tui.ts:168` `export const CURSOR_MARKER = "\x1b_pi:c\x07"`）。
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";
/// 隐藏/显示光标（`terminal.ts:501` / `:505`）。
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

/// 渲染失败（对应上游 `doRender` 里"行宽超过终端宽度"的抛错，`tui-main-screen.ts:534-548`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRenderError {
    /// 某行可见宽度超过终端宽度（上游会写 crash log 后抛错）。
    LineTooWide {
        /// 行号（0 起）。
        index: usize,
        /// 终端宽度。
        width: usize,
        /// 该行可见宽度。
        line_width: usize,
    },
}

/// 渲染状态：字段与上游 `TuiMainScreen` 的私有字段一一对应。
///
/// 行号均为 `ref/pi-0.87.1/packages/tui/src/tui-main-screen.ts`。
#[derive(Clone, Debug, Default)]
pub struct DiffState {
    /// ↔ `previousLines`（`:126`）：上一帧渲染出的行（含行尾 `SEGMENT_RESET`）。
    pub previous_lines: Vec<String>,
    /// ↔ `previousWidth`（`:128`）。
    pub previous_width: usize,
    /// ↔ `previousHeight`（`:129`）。
    pub previous_height: usize,
    /// ↔ `cursorRow`（`:130`）：内容末尾行（用于视口计算）。
    pub cursor_row: usize,
    /// ↔ `hardwareCursorRow`（`:131`）：终端光标实际所在行。
    pub hardware_cursor_row: usize,
    /// 上一帧写出的光标**列**（0 基，来自 `position_hardware_cursor` 的 `\e[<col+1>G`）。
    /// ref 只跟踪行；列留着给调试开关 `PI_TUI_CURSOR_DUMP` 做逐帧行列对比（V6）。
    pub cursor_col: usize,
    /// ↔ `maxLinesRendered`（`:132`）：工作区历史最大行数（缩小时用于 `clearOnShrink`）。
    pub max_lines_rendered: usize,
    /// 上一帧的**行样式**（P8d/C4）：样式变了也要重画该行，否则那一行会停在
    /// 旧样式上（实测 `[Context]` 之类的行在滚动后仍是默认色）。
    pub previous_styles: Vec<Option<crate::style::LineStyle>>,
    /// 上一帧的**行内分段样式**（`spans` 非空时优先于整行样式）。
    pub previous_spans: Vec<Vec<crate::style::Span>>,
    /// ↔ `previousViewportTop`（`:133`）。
    pub previous_viewport_top: usize,
    /// ↔ `TuiBase.clearOnShrink`（`tui.ts:482`，默认 `false`）。
    pub clear_on_shrink: bool,
    /// 下一次渲染**强制整屏重绘并清屏**（挂起恢复后屏幕可能已被别的内容覆盖）。
    pub force_clear: bool,
    /// ↔ ref 的 `hasOverlayEntries`（`tui-main-screen.ts:369-373`）：有浮层时**不**走
    /// clearOnShrink 的整屏重绘。由调用方每帧写入。
    pub has_overlay: bool,
    /// ↔ `TuiBase.fullRedrawCount`（`:481`）：整屏重绘次数（诊断用）。
    pub full_redraw_count: usize,
    /// ↔ `TuiBase.showHardwareCursor`（`tui.ts:485`，默认 `false`）：决定定位后写 `\e[?25h` 还是 `\e[?25l`。
    pub show_hardware_cursor: bool,
}

impl DiffState {
    /// 新建空状态（等价于构造后 `resetRenderState()` 之前的初值）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// 文本归一化（`utils.ts:387-412` `normalizeTerminalOutput`）：泰文/老挝文 AM 分解 + 制表符展开。
///
/// 说明：上游的制表符展开会跳过 ANSI 序列内部（`extractAnsiCode`），本实现逐字符替换；
/// 二者仅在"ANSI 序列内部含 `\t`"时不同，夹具中无此情形（见 `notes/progress.md` 的待验证项）。
fn normalize_terminal_output(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\u{0e33}' => out.push_str("\u{0e4d}\u{0e32}"),
            '\u{0eb3}' => out.push_str("\u{0ecd}\u{0eb2}"),
            '\t' => out.push_str("   "),
            other => out.push(other),
        }
    }
    out
}

/// 可见宽度（`utils.ts` 的 `visibleWidth`）：**跳过 ANSI 序列**后按 `unicode-width` 计量。
///
/// 上游 `visibleWidth` 会剔除 CSI/OSC/APC 序列；不剔除会把 `\x1b[31m` 这类序列的字节算进宽度，
/// 从而误判超宽（夹具里的 SGR 内容即暴露此点）。
fn visible_width(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut visible = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            match bytes.get(i + 1) {
                Some(b'[') => {
                    // CSI：ESC [ 参数... 终止字节（@..~ 的字母类；上游用 [mGKHJ]）
                    let mut j = i + 2;
                    while j < bytes.len() && !matches!(bytes[j], b'm' | b'G' | b'K' | b'H' | b'J') {
                        j += 1;
                    }
                    i = j + 1;
                    continue;
                }
                Some(b']') | Some(b'_') => {
                    // OSC / APC：到 BEL 或 ST(ESC \) 为止
                    let mut j = i + 2;
                    while j < bytes.len() {
                        if bytes[j] == 0x07 {
                            j += 1;
                            break;
                        }
                        if bytes[j] == 0x1b && bytes.get(j + 1) == Some(&b'\\') {
                            j += 2;
                            break;
                        }
                        j += 1;
                    }
                    i = j;
                    continue;
                }
                Some(_) | None => {
                    i += 2;
                    continue;
                }
            }
        }
        // 取一个完整 char（多字节安全）
        let rest = &text[i..];
        let ch = rest.chars().next().unwrap_or(' ');
        visible.push(ch);
        i += ch.len_utf8();
    }
    UnicodeWidthStr::width(visible.as_str())
}

/// `computeLineDiff`（`tui-main-screen.ts:258-262`）：目标行与当前屏幕行之差（正=下移，负=上移）。
/// 补位空格的样式：**只保留背景色**，不要前景/粗体/斜体。
///
/// ref `utils.ts:1050-1058` 的 padding 是纯空格，只被 `bgFn` 染色；把整行样式
/// （含前景与粗体）铺到补位空格上会多出成片的 bold/fg 差异（T06 实测 96 格）。
fn padding_style(style: crate::style::LineStyle) -> crate::style::LineStyle {
    crate::style::LineStyle::with_bg_only_tuple(style.bg)
}

/// 把行尾的**控制序列**（`SGR` 复位、`OSC 8` 关闭）与可见文本分开。
///
/// 依据 ref `utils.ts:1050-1058` `applyBackgroundToLine`：先按可见宽度补空格，
/// 再整体套 `bgFn`。我们的渲染行尾自带 `\x1b[0m` / `\x1b]8;;\x07`，
/// 若直接 `prefix + line + padding`，补位空格会落在行内复位之后 ⇒ 底色丢失
/// （S2R 实测：TS 行内无复位，补位空格紧跟 `48;2;…`）。
fn split_trailing_controls(line: &str) -> (&str, &str) {
    let bytes = line.as_bytes();
    let mut end = bytes.len();
    loop {
        if end == 0 {
            break;
        }
        // 结尾是 OSC（ESC ] … BEL）？
        if bytes[end - 1] == 0x07 {
            if let Some(start) = line[..end].rfind('\u{1b}') {
                let candidate = &line[start..end];
                if candidate.starts_with("\u{1b}]") {
                    end = start;
                    continue;
                }
            }
        }
        // 结尾是 CSI 序列（ESC [ … 终止字节）？
        if bytes[end - 1].is_ascii_alphabetic() {
            if let Some(start) = line[..end].rfind("\u{1b}[") {
                let candidate = &line[start..end];
                if candidate.starts_with("\u{1b}[")
                    && candidate.len() > 2
                    && candidate[2..candidate.len() - 1]
                        .chars()
                        .all(|ch| ch.is_ascii_digit() || ch == ';' || ch == '?')
                {
                    // 只剥离 **复位/样式**（以 `m` 结尾）；光标移动等不动。
                    if candidate.ends_with('m') {
                        end = start;
                        continue;
                    }
                }
            }
        }
        break;
    }
    (&line[..end], &line[end..])
}

/// 按 span 写一行：逐段 SGR + 文本，段间复位；最后按需补背景空格到整宽。
/// 把字节下标收敛到**字符边界**（向下取整）：span 的偏移由调用方计算，
/// 只要能落进多字节字符中间（例如按字节步进的 run-length 切段），这里兜底。
fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// 向上取整到字符边界（不越过文本末尾）。
fn ceil_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn write_spans(out: &mut String, line: &str, spans: &[crate::style::Span], width: usize) {
    let mut cursor = 0_usize;
    let mut last_background = None;
    for span in spans {
        if span.end <= cursor {
            continue;
        }
        if span.start > cursor {
            // P8e-16 修复：span 的字节偏移可能落在多字节字符中间
            // （真机崩溃：`end byte index 79 is not a char boundary; inside '…'`），
            // 切片前统一收敛到字符边界。
            let gap_end = ceil_char_boundary(line, span.start);
            out.push_str(&line[cursor..gap_end]);
        }
        let start = floor_char_boundary(line, span.start.max(cursor));
        if start >= line.len() {
            break;
        }
        let end = ceil_char_boundary(line, span.end.min(line.len()));
        out.push_str(&span.style.prefix());
        out.push_str(&line[start..end]);
        out.push_str(span.style.reset());
        if span.style.bg.is_some() {
            last_background = Some(span.style);
        }
        cursor = end;
    }
    if cursor < line.len() {
        out.push_str(&line[cursor..]);
    }
    // 末尾补空格：背景色或 `dim` 属性在 TS 里覆盖到整行（Text 的 padding/属性）。
    if let Some(style) = last_background {
        let used = visible_width(line);
        if used < width {
            let pad = padding_style(style);
            out.push_str("\u{1b}[0m");
            out.push_str(&pad.prefix());
            out.push_str(&" ".repeat(width - used));
            out.push_str(pad.reset());
        }
    }
}

fn compute_line_diff(
    target_row: i64,
    hardware_cursor_row: i64,
    prev_viewport_top: i64,
    viewport_top: i64,
) -> i64 {
    (target_row - viewport_top) - (hardware_cursor_row - prev_viewport_top)
}

fn push_move(out: &mut String, delta: i64) {
    if delta > 0 {
        out.push_str(&format!("\x1b[{delta}B"));
    } else if delta < 0 {
        out.push_str(&format!("\x1b[{}A", -delta));
    }
}

/// 整屏重绘（带样式）。`width` 为 0 时按"不补背景"处理。
fn full_render_styled(
    out: &mut String,
    lines: &[String],
    styles: &[Option<crate::style::LineStyle>],
    spans: &[Vec<crate::style::Span>],
    width: usize,
    clear: bool,
    height: usize,
) {
    // 整屏重绘必须**把全部行重写一遍**以重建回滚（ref `tui-main-screen.ts:285-300`）：
    // 只写视口会让清屏后的历史彻底消失（真机反馈"上翻不了历史，完全没法用"）。
    // 超大缓冲下这一帧的开销（35 万行 ≈ 38 MB）只能靠"减少缓冲行数"解决，不能靠截断输出。
    let _ = height;
    out.push_str(SYNC_START);
    if clear {
        out.push_str(CLEAR_ALL);
    }
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        let line_spans = spans.get(i).map(Vec::as_slice).unwrap_or(&[]);
        if !line_spans.is_empty() {
            write_spans(out, line, line_spans, width.max(visible_width(line)));
            continue;
        }
        match styles.get(i).and_then(|slot| slot.as_ref()) {
            Some(style) => {
                // 与增量路径同口径：前导空格不染色；背景色块整行覆盖、
                // 且**补位空格要写在行内复位之前**（ref `utils.ts:1050-1058`）。
                let (visible, trailing): (&str, &str) = if style.bg.is_some() {
                    split_trailing_controls(line)
                } else {
                    (line.as_str(), "")
                };
                let lead = visible.len() - visible.trim_start().len();
                // 前导空格**永远只带底色**：有前景色时也不给它们上色
                // （T75 真彩逐格实测：bash 输出行首那个空格 TS 是 fg=null + 底色）。
                if lead > 0 {
                    if style.bg.is_some() {
                        let pad = padding_style(*style);
                        out.push_str(&pad.prefix());
                        out.push_str(&visible[..lead]);
                        out.push_str(pad.reset());
                    } else {
                        out.push_str(&visible[..lead]);
                    }
                }
                out.push_str(&style.prefix());
                out.push_str(&visible[lead..]);
                if style.bg.is_some() {
                    let used = visible_width(visible);
                    if used < width {
                        let pad = padding_style(*style);
                        out.push_str("\u{1b}[0m");
                        out.push_str(&pad.prefix());
                        out.push_str(&" ".repeat(width - used));
                        out.push_str(pad.reset());
                    }
                }
                out.push_str(style.reset());
                out.push_str(trailing);
            }
            None => out.push_str(line),
        }
    }
    out.push_str(SYNC_END);
}

/// 从渲染行中提取光标位置并从行内剥除标记（`tui.ts:1382-1400` `extractCursorPosition`）。
///
/// 只扫描**末 `height` 行**（可见视口）；`row` 为行下标、`col` 为该行标记之前的可见宽度。
/// 返回 `(位置, 行数组是否被改动)`。
#[must_use]
pub fn extract_cursor_position(lines: &mut [String], height: usize) -> Option<(usize, usize)> {
    extract_cursor_position_with_index(lines, height).map(|(row, col, _index)| (row, col))
}

/// 同 [`extract_cursor_position`]，额外返回被剥掉的标记在该行内的**字节下标**。
///
/// 用途：span 的偏移是在**带标记**的行上算出来的（`header_spans` 在 `marker_apply` 之后跑），
/// 剥掉 7 字节标记后必须把标记之后的 span 偏移整体左移，否则切片会落进行尾的
/// `SEGMENT_RESET` 里 —— 真机 `;;` / `0m` 残留就是这个（2026-10-31 由 `PI_TUI_OUTPUT_DUMP`
/// 取证：`…\e]8` + `\e[0m` + `;;\a`，即 OSC 8 被 span 边界切成两半）。
#[must_use]
pub fn extract_cursor_position_with_index(
    lines: &mut [String],
    height: usize,
) -> Option<(usize, usize, usize)> {
    let start = lines.len().saturating_sub(height);
    for (row, line) in lines.iter_mut().enumerate().skip(start) {
        if let Some(index) = line.find(CURSOR_MARKER) {
            let prefix = &line[..index];
            let col = visible_width(prefix);
            let mut stripped = String::with_capacity(line.len());
            stripped.push_str(prefix);
            stripped.push_str(&line[index + CURSOR_MARKER.len()..]);
            *line = stripped;
            return Some((row, col, index));
        }
    }
    None
}

/// 把"带标记行"上的字节偏移换算到"已剥标记行"上（标记之后的偏移左移标记长度）。
fn shift_past_marker(offset: usize, marker_index: usize) -> usize {
    let marker_len = CURSOR_MARKER.len();
    if offset > marker_index + marker_len {
        offset - marker_len
    } else if offset > marker_index {
        // 落在标记自身范围内的偏移：收敛到标记起点（等价于"该字符已被标记占据"）。
        marker_index
    } else {
        offset
    }
}

/// 生成光标定位字节（`tui-main-screen.ts:623-660` `positionHardwareCursor`）。
///
/// - 无光标或 `total_lines == 0` ⇒ 只隐藏光标；
/// - 否则把行夹到 `[0, total_lines-1]`、列夹到 `>= 0`，写行位移（`\e[nA/B`）+ 绝对列（`\e[{col+1}G`），
///   再按 `show_hardware_cursor` 写 `\e[?25h` / `\e[?25l`。
///   同时返回更新后的 `hardware_cursor_row`。
fn position_hardware_cursor(
    state: &mut DiffState,
    cursor_pos: Option<(usize, usize)>,
    total_lines: usize,
) -> String {
    let mut out = String::new();
    let Some((row, col)) = cursor_pos else {
        out.push_str(HIDE_CURSOR);
        return out;
    };
    if total_lines == 0 {
        out.push_str(HIDE_CURSOR);
        return out;
    }
    let target_row = row.min(total_lines - 1);
    let target_col = col;
    state.cursor_col = target_col;
    let row_delta = target_row as i64 - state.hardware_cursor_row as i64;
    push_move(&mut out, row_delta);
    out.push_str(&format!("\x1b[{}G", target_col + 1));
    state.hardware_cursor_row = target_row;
    out.push_str(if state.show_hardware_cursor {
        SHOW_CURSOR
    } else {
        HIDE_CURSOR
    });
    out
}

/// 主屏差分渲染（`TuiMainScreen.doRender` 的纯函数版本，`tui-main-screen.ts:247-618`）。
///
/// 输入：状态、合成好的行数组（未加行尾复位）、终端宽高；输出：要写往终端的字节串（UTF-8 字符串）。
/// 不做 I/O、不写日志、不处理 kitty 图片与覆盖层（上游在这些分支上另有副作用，本移植未含，见文档）。
///
/// # Errors
/// 任一行可见宽度 > `width` 时返回 [`DiffRenderError::LineTooWide`]（对应上游抛错路径）。
pub fn diff_render(
    state: &mut DiffState,
    lines: &[String],
    width: usize,
    height: usize,
) -> Result<String, DiffRenderError> {
    // v1 夹具的假终端**忽略** `hideCursor/showCursor` ⇒ v1 的期望字节里不含光标序列；
    // 因此默认入口只返回渲染字节（行为与扩展前逐字节相同），光标字节由
    // [`diff_render_with_cursor`] 单独给出（W2 光标夹具用它比对）。
    let (out, _cursor) = diff_render_with_cursor(state, lines, width, height)?;
    Ok(out)
}

/// 同 [`diff_render`]，但额外返回**光标定位字节**（`positionHardwareCursor` 的输出，`tui-main-screen.ts:623-660`）。
///
/// 顺序与上游一致：先 `extractCursorPosition`（`:270`）→ 渲染 → `positionHardwareCursor`（`:614`、`:448` 等）。
/// 调用方应按 `render_bytes + cursor_bytes` 的顺序写往终端。
///
/// # Errors
/// 同 [`diff_render`]。
/// 带**整行样式**的差分渲染（P8d/C4）：文本与增量逻辑完全复用，
/// 只在"写这一行"时在行首加 SGR、行尾复位。宽度仍按纯文本计算。
pub fn diff_render_with_cursor_styled(
    state: &mut DiffState,
    lines: &[String],
    styles: &[Option<crate::style::LineStyle>],
    spans: &[Vec<crate::style::Span>],
    width: usize,
    height: usize,
) -> Result<(String, String), DiffRenderError> {
    let mut owned: Vec<String> = lines.to_vec();
    let found = extract_cursor_position_with_index(&mut owned, height);
    let cursor_pos = found.map(|(row, col, _index)| (row, col));
    // 标记（7 字节）已从该行剥掉，而 span 偏移是按"带标记"的行算的 ⇒ 把标记之后的
    // 偏移整体减 7，否则行尾 `SEGMENT_RESET` 会被 span 边界切开（`;;` / `0m` 残留）。
    let shifted_spans: Option<Vec<Vec<crate::style::Span>>> = found.and_then(|(row, _, index)| {
        let mut copy = spans.to_vec();
        let row_spans = copy.get_mut(row)?;
        for span in row_spans.iter_mut() {
            span.start = shift_past_marker(span.start, index);
            span.end = shift_past_marker(span.end, index);
        }
        Some(copy)
    });
    let spans = shifted_spans.as_deref().unwrap_or(spans);
    let out = diff_render_body_styled(state, &owned, styles, spans, width, height)?;
    dump_debug_frame(state, &owned, width, height);
    let cursor_bytes = position_hardware_cursor(state, cursor_pos, owned.len());
    Ok((out, cursor_bytes))
}

pub fn diff_render_with_cursor(
    state: &mut DiffState,
    lines: &[String],
    width: usize,
    height: usize,
) -> Result<(String, String), DiffRenderError> {
    let mut owned: Vec<String> = lines.to_vec();
    let cursor_pos = extract_cursor_position(&mut owned, height);
    let out = diff_render_body(state, &owned, width, height)?;
    dump_debug_frame(state, &owned, width, height);
    let cursor_bytes = position_hardware_cursor(state, cursor_pos, owned.len());
    Ok((out, cursor_bytes))
}

/// 与 ref `tui-main-screen.ts:569-598` 同款的调试转储（`PI_TUI_DEBUG=1` 才写）。
///
/// ref 写到 `/tmp/tui/render-<ts>-<rand>.log`，字段含 `viewportTop` / `newLines.length`
/// 与 `=== newLines ===` 的完整缓冲；这里只对齐**定位视口问题需要的**那几个字段，
/// 同样是"环境变量没开就完全不做事"。
fn dump_debug_frame(state: &DiffState, lines: &[String], width: usize, height: usize) {
    if std::env::var("PI_TUI_DEBUG").ok().as_deref() != Some("1") {
        return;
    }
    let directory = std::path::Path::new("/tmp/tui");
    if std::fs::create_dir_all(directory).is_err() {
        return;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let path = directory.join(format!("pi-rs-render-{stamp}.log"));
    let Ok(mut file) = std::fs::File::create(&path) else {
        return;
    };
    use std::io::Write as _;
    let _ = writeln!(file, "viewportTop: {}", state.previous_viewport_top);
    let _ = writeln!(file, "cursorRow: {}", state.cursor_row);
    let _ = writeln!(file, "width: {width}");
    let _ = writeln!(file, "height: {height}");
    let _ = writeln!(file, "newLines.length: {}", lines.len());
    let _ = writeln!(file, "previousLines.length: {}", state.previous_lines.len());
    let _ = writeln!(file, "maxLinesRendered: {}", state.max_lines_rendered);
    let _ = writeln!(file, "=== newLines ===");
    for (index, line) in lines.iter().enumerate() {
        let _ = writeln!(file, "{index}\t{line}");
    }
}

fn diff_render_body(
    state: &mut DiffState,
    lines: &[String],
    width: usize,
    height: usize,
) -> Result<String, DiffRenderError> {
    diff_render_body_styled(state, lines, &[], &[], width, height)
}

/// `PI_TUI_DEBUG_REDRAW=1` 时把"整屏重绘的原因"追加到 `/tmp/pi-tui-debug.log`
/// （ref `tui-main-screen.ts:320-327` 的 `logRedraw`；排查"全屏刷新/卡顿"只看这一行就够）。
fn log_redraw(reason: &str, previous: usize, new: usize, height: usize) {
    if std::env::var_os("PI_TUI_DEBUG_REDRAW").is_none() {
        return;
    }
    use std::io::Write as _;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/pi-tui-debug.log")
    {
        let _ = writeln!(
            file,
            "fullRender: {reason} (prev={previous}, new={new}, height={height})"
        );
    }
}

fn diff_render_body_styled(
    state: &mut DiffState,
    lines: &[String],
    styles: &[Option<crate::style::LineStyle>],
    spans: &[Vec<crate::style::Span>],
    width: usize,
    height: usize,
) -> Result<String, DiffRenderError> {
    let mut out = String::new();
    let width_changed = state.previous_width != 0 && state.previous_width != width;
    let height_changed = state.previous_height != 0 && state.previous_height != height;
    let previous_buffer_length = if state.previous_height > 0 {
        state.previous_viewport_top + state.previous_height
    } else {
        height
    };
    let mut prev_viewport_top = if height_changed {
        previous_buffer_length.saturating_sub(height)
    } else {
        state.previous_viewport_top
    };
    let mut viewport_top = prev_viewport_top;
    let mut hardware_cursor_row = state.hardware_cursor_row;

    // applyLineResets（`tui.ts:1353-1358`）：每行归一化后接 SEGMENT_RESET。
    let new_lines: Vec<String> = lines
        .iter()
        .map(|line| format!("{}{SEGMENT_RESET}", normalize_terminal_output(line)))
        .collect();

    // 首次渲染：不清屏，直接整屏输出（`:345-350`）。
    // 挂起恢复等场景：屏幕可能已被别的内容覆盖，必须清屏后整屏重绘，
    // 不能走"首帧假定屏幕干净"的分支（`tui-main-screen.ts:337-341`）。
    if state.force_clear {
        log_redraw(
            "force_clear",
            state.previous_lines.len(),
            new_lines.len(),
            height,
        );
        state.force_clear = false;
        full_render_styled(&mut out, &new_lines, styles, spans, width, true, height);
        state.full_redraw_count += 1;
        state.cursor_row = new_lines.len().saturating_sub(1);
        state.hardware_cursor_row = state.cursor_row;
        state.max_lines_rendered = new_lines.len();
        state.previous_viewport_top = new_lines.len().max(height).saturating_sub(height);
        state.previous_lines = new_lines;
        state.previous_styles = styles.to_vec();
        state.previous_spans = spans.to_vec();
        state.previous_width = width;
        state.previous_height = height;
        return Ok(out);
    }

    if state.previous_lines.is_empty() && !width_changed && !height_changed {
        log_redraw("first render", 0, new_lines.len(), height);
        full_render_styled(&mut out, &new_lines, styles, spans, width, false, height);
        state.full_redraw_count += 1;
        state.cursor_row = new_lines.len().saturating_sub(1);
        state.hardware_cursor_row = state.cursor_row;
        state.max_lines_rendered = new_lines.len();
        state.previous_viewport_top = new_lines.len().max(height).saturating_sub(height);
        state.previous_lines = new_lines;
        state.previous_styles = styles.to_vec();
        state.previous_spans = spans.to_vec();
        state.previous_width = width;
        state.previous_height = height;
        return Ok(out);
    }

    // 宽度变化必然整屏重绘（换行位置变化，`:357-361`）。
    // 高度变化（非 Termux）同样整屏重绘（`:366-371`；Termux 判断在纯函数中不参与，见文档）。
    // clearOnShrink：内容比历史最大行数短且无覆盖层时整屏重绘（`:375-379`）。
    if width_changed || height_changed {
        log_redraw(
            if width_changed {
                "width changed"
            } else {
                "height changed"
            },
            state.previous_lines.len(),
            new_lines.len(),
            height,
        );
        full_render_styled(&mut out, &new_lines, styles, spans, width, true, height);
        state.full_redraw_count += 1;
        state.cursor_row = new_lines.len().saturating_sub(1);
        state.hardware_cursor_row = state.cursor_row;
        state.max_lines_rendered = new_lines.len();
        state.previous_viewport_top = new_lines.len().max(height).saturating_sub(height);
        state.previous_lines = new_lines;
        state.previous_styles = styles.to_vec();
        state.previous_spans = spans.to_vec();
        state.previous_width = width;
        state.previous_height = height;
        return Ok(out);
    }

    // ref `tui-main-screen.ts:369-373`：`getClearOnShrink() && newLines.length < maxLinesRendered
    // && !hasOverlayEntries`。浮层（我们的 panel）存在时**不**走整屏重绘——否则会多清一次屏，
    // 把 `previous_viewport_top` 归 0 而后续帧的落点随之整体偏移。
    let has_overlay = state.has_overlay;
    if state.clear_on_shrink && new_lines.len() < state.max_lines_rendered && !has_overlay {
        log_redraw(
            "clearOnShrink",
            state.previous_lines.len(),
            new_lines.len(),
            height,
        );
        full_render_styled(&mut out, &new_lines, styles, spans, width, true, height);
        state.full_redraw_count += 1;
        state.cursor_row = new_lines.len().saturating_sub(1);
        state.hardware_cursor_row = state.cursor_row;
        state.max_lines_rendered = new_lines.len();
        state.previous_viewport_top = new_lines.len().max(height).saturating_sub(height);
        state.previous_lines = new_lines;
        state.previous_styles = styles.to_vec();
        state.previous_spans = spans.to_vec();
        state.previous_width = width;
        state.previous_height = height;
        return Ok(out);
    }

    // 找出首/末变化行（`:382-400`）。
    let mut first_changed: Option<usize> = None;
    let mut last_changed: usize = 0;
    let max_lines = new_lines.len().max(state.previous_lines.len());
    for i in 0..max_lines {
        let old_line = state.previous_lines.get(i).map_or("", String::as_str);
        let new_line = new_lines.get(i).map_or("", String::as_str);
        let old_style = state.previous_styles.get(i).copied().flatten();
        let new_style = styles.get(i).copied().flatten();
        // `None`（该行没有 span）与 `Some(空 vec)`（占位）语义相同，必须视为相等，
        // 否则 `previous_spans` 与 `spans` 长度不一致时，边界行会被永久判成"变化"，
        // 于是每帧都从那一行重画到屏幕底部（错误状态输出量随历史平方增长）。
        let old_spans = state
            .previous_spans
            .get(i)
            .filter(|line_spans| !line_spans.is_empty());
        let new_spans = spans.get(i).filter(|line_spans| !line_spans.is_empty());
        if old_line != new_line || old_style != new_style || old_spans != new_spans {
            if first_changed.is_none() {
                first_changed = Some(i);
            }
            last_changed = i;
        }
    }
    let appended_lines = new_lines.len() > state.previous_lines.len();
    if appended_lines {
        if first_changed.is_none() {
            first_changed = Some(state.previous_lines.len());
        }
        last_changed = new_lines.len().saturating_sub(1);
    }

    // 没有变化：只更新高度（`:411-417`）。
    let Some(first_changed) = first_changed else {
        state.previous_viewport_top = prev_viewport_top;
        state.previous_height = height;
        return Ok(out);
    };
    let append_start =
        appended_lines && first_changed == state.previous_lines.len() && first_changed > 0;

    // 变化全落在"被删除的行"上：只清行不重画（`:420-455`）。
    if first_changed >= new_lines.len() {
        if state.previous_lines.len() > new_lines.len() {
            out.push_str(SYNC_START);
            let target_row = new_lines.len().saturating_sub(1);
            if target_row < prev_viewport_top {
                out.clear();
                full_render_styled(&mut out, &new_lines, styles, spans, width, true, height);
                state.full_redraw_count += 1;
                state.cursor_row = new_lines.len().saturating_sub(1);
                state.hardware_cursor_row = state.cursor_row;
                state.max_lines_rendered = new_lines.len();
                state.previous_viewport_top = new_lines.len().max(height).saturating_sub(height);
                state.previous_lines = new_lines;
                state.previous_styles = styles.to_vec();
                state.previous_spans = spans.to_vec();
                state.previous_width = width;
                state.previous_height = height;
                return Ok(out);
            }
            let line_diff = compute_line_diff(
                target_row as i64,
                hardware_cursor_row as i64,
                prev_viewport_top as i64,
                viewport_top as i64,
            );
            push_move(&mut out, line_diff);
            out.push('\r');
            let extra_lines = state.previous_lines.len() - new_lines.len();
            if extra_lines > height {
                log_redraw(
                    "extraLines > height",
                    state.previous_lines.len(),
                    new_lines.len(),
                    height,
                );
                out.clear();
                full_render_styled(&mut out, &new_lines, styles, spans, width, true, height);
                state.full_redraw_count += 1;
                state.cursor_row = new_lines.len().saturating_sub(1);
                state.hardware_cursor_row = state.cursor_row;
                state.max_lines_rendered = new_lines.len();
                state.previous_viewport_top = new_lines.len().max(height).saturating_sub(height);
                state.previous_lines = new_lines;
                state.previous_styles = styles.to_vec();
                state.previous_spans = spans.to_vec();
                state.previous_width = width;
                state.previous_height = height;
                return Ok(out);
            }
            let clear_start_offset = if new_lines.is_empty() { 0 } else { 1 };
            if extra_lines > 0 && clear_start_offset > 0 {
                out.push_str(&format!("\x1b[{clear_start_offset}B"));
            }
            for i in 0..extra_lines {
                out.push_str("\r\x1b[2K");
                if i + 1 < extra_lines {
                    out.push_str("\x1b[1B");
                }
            }
            let move_back = (extra_lines + clear_start_offset).saturating_sub(1);
            if move_back > 0 {
                out.push_str(&format!("\x1b[{move_back}A"));
            }
            out.push_str(SYNC_END);
            state.cursor_row = target_row;
            state.hardware_cursor_row = target_row;
        }
        commit_written_styles(state, styles, first_changed, first_changed);
        state.previous_lines = new_lines;
        state.previous_viewport_top = prev_viewport_top;
        state.previous_width = width;
        state.previous_height = height;
        return Ok(out);
    }

    // 首个变化行在**上一帧视口之上**：差分无法触达，整屏重绘（`:457-463`）。
    if first_changed < prev_viewport_top {
        log_redraw(
            "firstChanged above viewport",
            state.previous_lines.len(),
            new_lines.len(),
            height,
        );
        // 排障：把"视口之上"实际变化的行打出来（只在前 3 行差异，避免日志爆炸）。
        // 例：`/resume` 只多出完成框 2 行，但若判到 35 万行之前，说明中间有行被连带改变。
        if std::env::var_os("PI_TUI_DEBUG_REDRAW").is_some() {
            use std::io::Write as _;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/pi-tui-debug.log")
            {
                let mut shown = 0;
                for i in 0..max_lines {
                    if shown >= 3 {
                        break;
                    }
                    let old_line = state.previous_lines.get(i).map_or("", String::as_str);
                    let new_line = new_lines.get(i).map_or("", String::as_str);
                    let old_style = state.previous_styles.get(i).copied().flatten();
                    let new_style = styles.get(i).copied().flatten();
                    let old_spans = state
                        .previous_spans
                        .get(i)
                        .filter(|line_spans| !line_spans.is_empty());
                    let new_spans = spans.get(i).filter(|line_spans| !line_spans.is_empty());
                    if old_line != new_line || old_style != new_style || old_spans != new_spans {
                        let _ = writeln!(
                            file,
                            "  diff[{i}] line={} style={} spans={}\n    old={:?}\n    new={:?}",
                            old_line != new_line,
                            old_style != new_style,
                            old_spans != new_spans,
                            old_line.chars().take(60).collect::<String>(),
                            new_line.chars().take(60).collect::<String>()
                        );
                        let _ = writeln!(file, "    old_spans={:?}", old_spans);
                        let _ = writeln!(file, "    new_spans={:?}", new_spans);
                        shown += 1;
                    }
                }
            }
        }
        full_render_styled(&mut out, &new_lines, styles, spans, width, true, height);
        state.full_redraw_count += 1;
        state.cursor_row = new_lines.len().saturating_sub(1);
        state.hardware_cursor_row = state.cursor_row;
        state.max_lines_rendered = new_lines.len();
        state.previous_viewport_top = new_lines.len().max(height).saturating_sub(height);
        state.previous_lines = new_lines;
        state.previous_styles = styles.to_vec();
        state.previous_spans = spans.to_vec();
        state.previous_width = width;
        state.previous_height = height;
        return Ok(out);
    }

    // 增量渲染（`:465-570`）。
    out.push_str(SYNC_START);
    let prev_viewport_bottom = prev_viewport_top + height - 1;
    let move_target_row = if append_start {
        first_changed - 1
    } else {
        first_changed
    };
    if move_target_row > prev_viewport_bottom {
        let current_screen_row =
            (hardware_cursor_row.saturating_sub(prev_viewport_top)).min(height.saturating_sub(1));
        let move_to_bottom = height.saturating_sub(1).saturating_sub(current_screen_row);
        if move_to_bottom > 0 {
            out.push_str(&format!("\x1b[{move_to_bottom}B"));
        }
        let scroll = move_target_row - prev_viewport_bottom;
        for _ in 0..scroll {
            out.push_str("\r\n");
        }
        prev_viewport_top += scroll;
        viewport_top += scroll;
        hardware_cursor_row = move_target_row;
    }
    let line_diff = compute_line_diff(
        move_target_row as i64,
        hardware_cursor_row as i64,
        prev_viewport_top as i64,
        viewport_top as i64,
    );
    push_move(&mut out, line_diff);
    out.push_str(if append_start { "\r\n" } else { "\r" });

    let render_end = last_changed.min(new_lines.len().saturating_sub(1));
    for (i, line) in new_lines
        .iter()
        .enumerate()
        .take(render_end + 1)
        .skip(first_changed)
    {
        if i > first_changed {
            out.push_str("\r\n");
        }
        // 上游在此处会写 crash log 并抛错（`:534-548`）；本移植只返回错误。
        let width_of_line = visible_width(line);
        if width_of_line > width {
            return Err(DiffRenderError::LineTooWide {
                index: i,
                width,
                line_width: width_of_line,
            });
        }
        out.push_str("\x1b[2K");
        // 整行样式：行首 SGR、行尾复位（复位序列的宽度为 0，不影响 `visible_width`）。
        // 行内分段优先；没有分段时退回整行样式。
        let line_spans = spans.get(i).map(Vec::as_slice).unwrap_or(&[]);
        if line_spans.is_empty() {
            match styles.get(i).and_then(|slot| slot.as_ref()) {
                Some(style) => {
                    // ref 的 `Text(..., paddingX=1)` 里**前导空格不染色**；但**背景色块**
                    // 必须整行覆盖（用户消息的 padding 行就是纯空格行）。
                    // 另外按 `utils.ts:1050-1058`：补位空格必须在**行内复位之前**写，
                    // 否则底色会被行内 `\x1b[0m` 抵消（S2R 实测的根因）。
                    let (visible, trailing): (&str, &str) = if style.bg.is_some() {
                        split_trailing_controls(line)
                    } else {
                        (line.as_str(), "")
                    };
                    // 前导空格：无背景时**不染色**；有背景时**只染背景**（ref 的 padding
                    // 只有底色，文字才有前景色 —— T01 最后 1 格就是这么来的）。
                    let lead = visible.len() - visible.trim_start().len();
                    if lead > 0 {
                        if style.bg.is_some() {
                            let pad = padding_style(*style);
                            out.push_str(&pad.prefix());
                            out.push_str(&visible[..lead]);
                            out.push_str(pad.reset());
                        } else {
                            out.push_str(&visible[..lead]);
                        }
                    }
                    out.push_str(&style.prefix());
                    out.push_str(&visible[lead..]);
                    if style.bg.is_some() {
                        let used = visible_width(visible);
                        if used < width {
                            let pad = padding_style(*style);
                            out.push_str("\u{1b}[0m");
                            out.push_str(&pad.prefix());
                            out.push_str(&" ".repeat(width - used));
                            out.push_str(pad.reset());
                        }
                    }
                    out.push_str(style.reset());
                    out.push_str(trailing);
                }
                None => out.push_str(line),
            }
        } else {
            write_spans(&mut out, line, line_spans, width);
        }
    }

    let mut final_cursor_row = render_end;
    if state.previous_lines.len() > new_lines.len() {
        if render_end < new_lines.len().saturating_sub(1) {
            let move_down = new_lines.len() - 1 - render_end;
            out.push_str(&format!("\x1b[{move_down}B"));
            final_cursor_row = new_lines.len() - 1;
        }
        let extra_lines = state.previous_lines.len() - new_lines.len();
        for _ in new_lines.len()..state.previous_lines.len() {
            out.push_str("\r\n\x1b[2K");
        }
        out.push_str(&format!("\x1b[{extra_lines}A"));
    }
    out.push_str(SYNC_END);

    state.cursor_row = new_lines.len().saturating_sub(1);
    state.hardware_cursor_row = final_cursor_row;
    state.max_lines_rendered = state.max_lines_rendered.max(new_lines.len());
    state.previous_viewport_top =
        prev_viewport_top.max((final_cursor_row + 1).saturating_sub(height));
    // **只提交实际写出的行**的样式：未写出的行保持"屏幕上的旧样式"，
    // 这样"算出来变了但这一帧没写"的行下一帧还能被检测到并补画
    // （S1 实测：用户消息块的底色就是因为这个而永远没写上屏）。
    commit_written_styles(state, styles, first_changed, render_end);
    state.previous_lines = new_lines;
    state.previous_width = width;
    state.previous_height = height;
    Ok(out)
}

/// 把 `[from, to]` 这段行的样式记为"已写出"，其余行保留旧值。
fn commit_written_styles(
    state: &mut DiffState,
    styles: &[Option<crate::style::LineStyle>],
    from: usize,
    to: usize,
) {
    if let Some(path) = std::env::var_os("PI_TUI_STYLE_DUMP") {
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(
                file,
                "WRITE from={from} to={to} viewport_top={} lines={}",
                state.previous_viewport_top,
                styles.len()
            );
        }
    }
    state.previous_styles.resize(styles.len(), None);
    if styles.is_empty() || from >= styles.len() {
        return;
    }
    let last = to.min(styles.len() - 1);
    for index in from..=last {
        state.previous_styles[index] = styles.get(index).copied().flatten();
    }
}

impl std::fmt::Display for DiffRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LineTooWide {
                index,
                width,
                line_width,
            } => write!(
                f,
                "rendered line {index} exceeds terminal width ({line_width} > {width})"
            ),
        }
    }
}

impl std::error::Error for DiffRenderError {}

#[cfg(test)]
mod tests {
    use super::{diff_render, DiffRenderError, DiffState};
    use std::path::PathBuf;

    /// 夹具侧车路径（`tools/difftest/tui-oracle/fixtures.lines.txt`，由 `gen.mjs` 与 JSON 同时生成）。
    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/difftest/tui-oracle/fixtures.lines.txt")
    }

    fn decode_base64(input: &str) -> Vec<u8> {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::with_capacity(input.len() / 4 * 3);
        let mut acc: u32 = 0;
        let mut bits = 0;
        for byte in input.bytes() {
            if byte == b'=' {
                break;
            }
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
        out
    }

    struct Frame {
        width: usize,
        height: usize,
        lines: Vec<String>,
        out: Vec<u8>,
    }

    struct Case {
        id: String,
        frames: Vec<Frame>,
    }

    fn load_cases() -> Vec<Case> {
        let text = std::fs::read_to_string(fixture_path()).expect("读取夹具侧车失败");
        let mut cases: Vec<Case> = Vec::new();
        let mut pending: Option<Frame> = None;
        for line in text.lines() {
            let mut parts = line.split(' ');
            match parts.next() {
                Some("case") => {
                    let id = parts.next().unwrap_or_default().to_string();
                    cases.push(Case {
                        id,
                        frames: Vec::new(),
                    });
                }
                Some("frame") => {
                    let width: usize = parts.next().unwrap_or("0").parse().unwrap_or(0);
                    let height: usize = parts.next().unwrap_or("0").parse().unwrap_or(0);
                    pending = Some(Frame {
                        width,
                        height,
                        lines: Vec::new(),
                        out: Vec::new(),
                    });
                }
                Some("line") => {
                    let data = parts.next().unwrap_or_default();
                    let decoded = decode_base64(data);
                    if let Some(frame) = pending.as_mut() {
                        frame
                            .lines
                            .push(String::from_utf8(decoded).expect("夹具行不是 UTF-8"));
                    }
                }
                Some("out") => {
                    let data = parts.next().unwrap_or_default();
                    if let (Some(frame), Some(case)) = (pending.take(), cases.last_mut()) {
                        let mut frame = frame;
                        frame.out = decode_base64(data);
                        case.frames.push(frame);
                    }
                }
                _ => {}
            }
        }
        cases
    }

    /// 失败时输出用例 id、帧号、首个差异字节偏移与前后各 32 字节。
    #[test]
    fn fixtures_match_ts_oracle_byte_for_byte() {
        let cases = load_cases();
        assert!(cases.len() >= 210, "夹具用例应 ≥210，实际 {}", cases.len());
        let mut failures: Vec<String> = Vec::new();
        for case in &cases {
            let mut state = DiffState::new();
            for (index, frame) in case.frames.iter().enumerate() {
                let actual = match diff_render(&mut state, &frame.lines, frame.width, frame.height)
                {
                    Ok(bytes) => bytes.into_bytes(),
                    Err(error) => {
                        failures.push(format!("{} 帧{index}: 返回错误 {error}", case.id));
                        break;
                    }
                };
                if actual == frame.out {
                    continue;
                }
                let offset = actual
                    .iter()
                    .zip(&frame.out)
                    .position(|(a, b)| a != b)
                    .unwrap_or(actual.len().min(frame.out.len()));
                let from = offset.saturating_sub(32);
                let to_actual = (offset + 32).min(actual.len());
                let to_expected = (offset + 32).min(frame.out.len());
                failures.push(format!(
                    "{} 帧{index}: 首个差异字节 offset={offset}\n  实际[{from}..{to_actual}]={:?}\n  期望[{from}..{to_expected}]={:?}",
                    case.id,
                    String::from_utf8_lossy(&actual[from..to_actual]),
                    String::from_utf8_lossy(&frame.out[from..to_expected]),
                ));
                break;
            }
        }
        assert!(
            failures.is_empty(),
            "夹具未全部通过（{} 个用例失败）：\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn same_state_same_input_is_deterministic() {
        let lines = vec!["a".to_string(), "b".to_string()];
        let mut first = DiffState::new();
        let mut second = DiffState::new();
        let a = diff_render(&mut first, &lines, 20, 10).expect("渲染失败");
        let b = diff_render(&mut second, &lines, 20, 10).expect("渲染失败");
        assert_eq!(a, b);
        let a2 = diff_render(&mut first, &lines, 20, 10).expect("渲染失败");
        let b2 = diff_render(&mut second, &lines, 20, 10).expect("渲染失败");
        assert_eq!(a2, b2);
    }

    #[test]
    fn zero_lines_renders_empty_but_syncs() {
        let mut state = DiffState::new();
        let out = diff_render(&mut state, &[], 10, 5).expect("渲染失败");
        assert_eq!(out, "\x1b[?2026h\x1b[?2026l");
        assert!(state.previous_lines.is_empty());
    }

    #[test]
    fn width_one_truncation_is_caller_responsibility() {
        // 上游要求组件自行截断：宽度 1、内容 1 列可渲染；超过 1 列则报错（不 panic）。
        let mut state = DiffState::new();
        let ok = diff_render(&mut state, &["x".to_string()], 1, 3).expect("1 列应可渲染");
        assert!(ok.contains('x'));
        // 上游只在**增量渲染**路径检查宽度（首次整屏渲染不检查）⇒ 第二帧超宽才报错。
        let mut state2 = DiffState::new();
        diff_render(&mut state2, &["x".to_string()], 1, 3).expect("首帧 1 列");
        let error =
            diff_render(&mut state2, &["xy".to_string()], 1, 3).expect_err("超宽应返回错误");
        assert_eq!(
            error,
            DiffRenderError::LineTooWide {
                index: 0,
                width: 1,
                line_width: 2
            }
        );
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::{diff_render_with_cursor, DiffState};
    use std::path::PathBuf;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/difftest/tui-oracle/fixtures-cursor.lines.txt")
    }

    fn decode_base64(input: &str) -> Vec<u8> {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let (mut acc, mut bits) = (0u32, 0u32);
        for byte in input.bytes() {
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
        out
    }

    struct Frame {
        width: usize,
        height: usize,
        lines: Vec<String>,
        out: Vec<u8>,
    }

    struct Case {
        id: String,
        show_hardware_cursor: bool,
        frames: Vec<Frame>,
    }

    fn load_cases() -> Vec<Case> {
        let text = std::fs::read_to_string(fixture_path()).expect("读取光标夹具侧车失败");
        let mut cases: Vec<Case> = Vec::new();
        let mut pending: Option<Frame> = None;
        for line in text.lines() {
            let mut parts = line.split(' ');
            match parts.next() {
                Some("case") => {
                    let id = parts.next().unwrap_or_default().to_string();
                    let _seed = parts.next();
                    let _frames = parts.next();
                    let show = parts.next() == Some("1");
                    cases.push(Case {
                        id,
                        show_hardware_cursor: show,
                        frames: Vec::new(),
                    });
                }
                Some("frame") => {
                    let width: usize = parts.next().unwrap_or("0").parse().unwrap_or(0);
                    let height: usize = parts.next().unwrap_or("0").parse().unwrap_or(0);
                    pending = Some(Frame {
                        width,
                        height,
                        lines: Vec::new(),
                        out: Vec::new(),
                    });
                }
                Some("line") => {
                    let decoded = decode_base64(parts.next().unwrap_or_default());
                    if let Some(frame) = pending.as_mut() {
                        frame
                            .lines
                            .push(String::from_utf8(decoded).expect("夹具行不是 UTF-8"));
                    }
                }
                Some("out") => {
                    let data = parts.next().unwrap_or_default();
                    if let (Some(mut frame), Some(case)) = (pending.take(), cases.last_mut()) {
                        frame.out = decode_base64(data);
                        case.frames.push(frame);
                    }
                }
                _ => {}
            }
        }
        cases
    }

    /// 光标夹具：`tools/difftest/tui-oracle/fixtures.json`（由 `gen.mjs` 以 ref 真实
    /// `TUI` 渲染生成，含每帧输出的 base64 原文）。
    #[test]
    fn cursor_fixtures_match_ts_oracle_byte_for_byte() {
        let cases = load_cases();
        assert!(cases.len() >= 112, "光标夹具应 ≥112，实际 {}", cases.len());
        let mut failures: Vec<String> = Vec::new();
        for case in &cases {
            let mut state = DiffState::new();
            state.show_hardware_cursor = case.show_hardware_cursor;
            for (index, frame) in case.frames.iter().enumerate() {
                let (render, cursor) = match diff_render_with_cursor(
                    &mut state,
                    &frame.lines,
                    frame.width,
                    frame.height,
                ) {
                    Ok(pair) => pair,
                    Err(error) => {
                        failures.push(format!("{} 帧{index}: 返回错误 {error}", case.id));
                        break;
                    }
                };
                let mut actual = render.into_bytes();
                actual.extend_from_slice(cursor.as_bytes());
                if actual == frame.out {
                    continue;
                }
                let offset = actual
                    .iter()
                    .zip(&frame.out)
                    .position(|(a, b)| a != b)
                    .unwrap_or(actual.len().min(frame.out.len()));
                let from = offset.saturating_sub(32);
                let to_actual = (offset + 32).min(actual.len());
                let to_expected = (offset + 32).min(frame.out.len());
                failures.push(format!(
                    "{} 帧{index}: 首个差异字节 offset={offset}\n  实际[{from}..{to_actual}]={:?}\n  期望[{from}..{to_expected}]={:?}",
                    case.id,
                    String::from_utf8_lossy(&actual[from..to_actual]),
                    String::from_utf8_lossy(&frame.out[from..to_expected]),
                ));
                break;
            }
        }
        assert!(
            failures.is_empty(),
            "光标夹具未全部通过（{} 个失败）：\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}

/// 退出前把光标移到内容末尾下方并换行（`ref/.../tui-main-screen.ts:168-176` `beforeTerminalStop`）。
///
/// 语义与 ref 逐字一致：无内容 ⇒ 空串；否则 `" "` + `\e[{delta}B|A`（`delta = previous_lines.len() - hardware_cursor_row`）+ `"\r\n"`。
/// `" "` 的作用是**占位**（让空白行上也有字符，确保末行换行时的滚动行为确定）。
#[must_use]
pub fn before_terminal_stop(state: &DiffState) -> String {
    if state.previous_lines.is_empty() {
        return String::new();
    }
    let mut out = String::from(" ");
    let target_row = state.previous_lines.len() as i64;
    let delta = target_row - state.hardware_cursor_row as i64;
    if delta > 0 {
        out.push_str(&format!("\x1b[{delta}B"));
    } else if delta < 0 {
        out.push_str(&format!("\x1b[{}A", -delta));
    }
    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod exit_tests {
    use super::{before_terminal_stop, diff_render_with_cursor, DiffState};
    use std::path::PathBuf;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/difftest/tui-oracle/fixtures-exit.lines.txt")
    }

    fn decode_base64(input: &str) -> Vec<u8> {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let (mut acc, mut bits) = (0u32, 0u32);
        for byte in input.bytes() {
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
        out
    }

    struct Case {
        id: String,
        show_hardware_cursor: bool,
        frames: Vec<(usize, usize, Vec<String>)>,
        stop: Vec<u8>,
    }

    fn load_cases() -> Vec<Case> {
        let text = std::fs::read_to_string(fixture_path()).expect("读取退出序列夹具失败");
        let mut cases: Vec<Case> = Vec::new();
        let mut pending: Option<(usize, usize, Vec<String>)> = None;
        for line in text.lines() {
            let mut parts = line.split(' ');
            match parts.next() {
                Some("case") => {
                    let id = parts.next().unwrap_or_default().to_string();
                    let _seed = parts.next();
                    let _frames = parts.next();
                    let show = parts.next() == Some("1");
                    let stop = decode_base64(parts.next().unwrap_or_default());
                    cases.push(Case {
                        id,
                        show_hardware_cursor: show,
                        frames: Vec::new(),
                        stop,
                    });
                }
                Some("frame") => {
                    let width: usize = parts.next().unwrap_or("0").parse().unwrap_or(0);
                    let height: usize = parts.next().unwrap_or("0").parse().unwrap_or(0);
                    pending = Some((width, height, Vec::new()));
                }
                Some("line") => {
                    let decoded = decode_base64(parts.next().unwrap_or_default());
                    if let Some((_, _, lines)) = pending.as_mut() {
                        lines.push(String::from_utf8(decoded).expect("夹具行不是 UTF-8"));
                    }
                }
                Some("out") => {
                    let _ = parts.next();
                    if let (Some(frame), Some(case)) = (pending.take(), cases.last_mut()) {
                        case.frames.push(frame);
                    }
                }
                _ => {}
            }
        }
        cases
    }

    /// 读 `fixtures-exit.json` 的侧车，逐用例：跑完帧序列后调用 [`before_terminal_stop`] 并逐字节比对。
    #[test]
    fn exit_fixtures_match_ts_oracle_byte_for_byte() {
        let cases = load_cases();
        assert!(
            cases.len() >= 100,
            "退出序列夹具应 ≥100，实际 {}",
            cases.len()
        );
        let mut failures: Vec<String> = Vec::new();
        for case in &cases {
            let mut state = DiffState::new();
            state.show_hardware_cursor = case.show_hardware_cursor;
            for (width, height, lines) in &case.frames {
                if let Err(error) = diff_render_with_cursor(&mut state, lines, *width, *height) {
                    failures.push(format!("{}: 渲染错误 {error}", case.id));
                    break;
                }
            }
            let actual = before_terminal_stop(&state).into_bytes();
            if actual == case.stop {
                continue;
            }
            let offset = actual
                .iter()
                .zip(&case.stop)
                .position(|(a, b)| a != b)
                .unwrap_or(actual.len().min(case.stop.len()));
            failures.push(format!(
                "{}: 首个差异 offset={offset} 实际={:?} 期望={:?}",
                case.id,
                String::from_utf8_lossy(&actual)
                    .chars()
                    .take(40)
                    .collect::<String>(),
                String::from_utf8_lossy(&case.stop)
                    .chars()
                    .take(40)
                    .collect::<String>(),
            ));
        }
        assert!(
            failures.is_empty(),
            "退出序列夹具未全部通过（{} 个失败）：\n{}",
            failures.len(),
            failures
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[cfg(test)]
mod char_boundary_tests {
    use super::*;
    use crate::style::{LineStyle, Span};

    /// P8e-16 崩溃复现：span 的字节偏移落在多字节字符中间时不得 panic。
    /// 真机报错：`end byte index 79 is not a char boundary; it is inside '…'`。
    #[test]
    fn span_offsets_inside_multibyte_char_do_not_panic() {
        // '…' 是 3 字节（E2 80 A6），让 span 的起点/终点都落在它内部
        let line = "abc…def";
        let ellipsis_start = 3; // '…' 起始字节
        let spans = vec![
            Span::new(0, ellipsis_start + 1, LineStyle::default()), // 终点在 '…' 内
            Span::new(ellipsis_start + 2, line.len(), LineStyle::default()), // 起点在 '…' 内
        ];
        let mut out = String::new();
        write_spans(&mut out, line, &spans, 0);
        // 关键断言：不 panic，且文本内容不丢
        let plain = strip_sgr(&out);
        assert!(plain.contains("abc"), "{plain:?}");
        assert!(plain.contains("def"), "{plain:?}");
    }

    fn strip_sgr(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            if ch == '\u{1b}' {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
                continue;
            }
            out.push(ch);
        }
        out
    }
}

#[cfg(test)]
mod cursor_marker_span_tests {
    use super::{diff_render_with_cursor_styled, shift_past_marker, DiffState, CURSOR_MARKER};
    use crate::style::{LineStyle, Span};

    /// 真机取证的字节形状：行内插入了光标标记（7 字节）与反显转义（8 字节），
    /// 该行的 span 偏移是按**带标记**的行算出来的。剥掉标记后若不修正，
    /// 最后一个 span 的 `end` 会越进行尾 `SEGMENT_RESET` 的 OSC 8 里，
    /// 渲染出 `…\e]8` + `\e[0m` + `;;\a` —— 屏幕上就是多出来的 `;;`。
    /// 这条用例复刻 `PI_TUI_OUTPUT_DUMP` 里抓到的 frame 11 形状。
    #[test]
    fn spans_are_shifted_after_cursor_marker_is_stripped() {
        let mut line = String::from(" pi v");
        line.push_str(CURSOR_MARKER);
        line.push_str("\u{1b}[7m0\u{1b}[0m");
        line.push_str(".87.1");
        let with_marker_len = line.len();
        let spans = vec![
            Span::new(1, 3, LineStyle::fg((0x8a, 0xbe, 0xb7)).bold()),
            Span::new(3, with_marker_len, LineStyle::fg((0x80, 0x80, 0x80))),
        ];
        let mut state = DiffState::new();
        let (out, _cursor) =
            diff_render_with_cursor_styled(&mut state, &[line], &[None], &[spans], 80, 24)
                .expect("渲染");
        let opens = out.matches("\u{1b}]8").count();
        let closes = out.matches("\u{1b}]8;;\u{7}").count();
        assert_eq!(opens, closes, "OSC 8 被 span 边界切开：{out:?}");
        assert!(
            !out.contains("\u{1b}]8\u{1b}[0m"),
            "OSC 8 被 span 复位切断：{out:?}"
        );
        assert!(!out.contains(";;\u{7};;"), "出现重复残片：{out:?}");
    }

    /// 偏移换算的边界：标记之前不动、标记内部收敛到标记起点、标记之后左移标记长度。
    #[test]
    fn shift_past_marker_covers_all_three_zones() {
        let index = 10;
        let len = CURSOR_MARKER.len();
        assert_eq!(shift_past_marker(0, index), 0);
        assert_eq!(shift_past_marker(index, index), index);
        assert_eq!(shift_past_marker(index + 1, index), index);
        assert_eq!(shift_past_marker(index + len, index), index);
        assert_eq!(shift_past_marker(index + len + 1, index), index + 1);
    }
}
