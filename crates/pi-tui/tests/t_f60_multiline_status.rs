//! F60/2：**多行状态文本不得把整帧打掉**（真机 `/todo` 卡死的根因）。
//!
//! 现象（真机 + 真 pty 复现）：输入 `/todo` 回车后屏幕更新一次，之后**任何按键都没反应**，
//! 只能 Ctrl+D 强退。取证结果不是死锁：主线程在 `recv_timeout` 上正常空转、
//! `state.input` 也在随按键增长（`x` → `xy`），但 `diff_render` 每帧都返回
//! `Err(LineTooWide)`，`draw()` 把整帧丢弃（F57/D4 的"渲染错误不写终端"），
//! 于是**每个后续帧都是空的**——屏幕上永远停在那一帧之前。
//!
//! 超宽行来自扩展的 `ctx.ui.notify`：`pi-handoff-gate` 的 `/todo` 把清单
//! `renderTodos(state)`（**多行**字符串）交给 notify，宿主把它当 `Status` 条目。
//! `Status` 的渲染是"单行"路径：`clamp_to_width(&format!(" {text}"), cols)`。
//! `clamp_to_width` 用 `UnicodeWidthChar::width` 量宽，`\n` 的宽度是 **0**，
//! 而差分层的 `visible_width` 把 `\n` 当 **1** 列 ⇒ 同一行两侧量出 120 vs 122，
//! 判定 `122 > 120` 直接报错。
//!
//! 修法：`Status`/`Notice` 与助手正文一样按**行**逐行排版（`text.lines()`），
//! 每行单独 `clamp_to_width`/`wrap_to_width`。这样：① 多行文本按行显示（与 ref 的
//! `Text` 组件一致：它在 `wrapTextWithAnsi` 内部先 `split(/\r\n|\r|\n/)`）；
//! ② 帧里不再出现内嵌换行，差分层与排版层的宽度口径一

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_tui::diff::{diff_render_with_cursor_styled, DiffState};
use pi_tui::{render_all, render_all_marker_cached_styled, RenderCache, TranscriptItem, TuiState};

fn base_state() -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.quiet_startup = true;
    state
}

/// `pi-handoff-gate` 的 `/todo` 会 notify 一个**多行**清单（真实文本，含 CJK 与长英文行）。
const TODO_LIST: &str = "[~] #1 1a 真 pty 复现 /allcopy 的 require is not defined 并定位\n[ ] #2 1b 修 shim 为 ESM import，测试改走真实加载路径\n[ ]";

/// 帧里任何一行都不得含内嵌换行（排版层的宽度口径与差分层不一致的**直接来源**）。
#[test]
fn status_with_newlines_never_leaks_embedded_newlines() {
    let mut state = base_state();
    state
        .transcript
        .push(TranscriptItem::Status(TODO_LIST.to_string()));
    let lines = render_all(&state, 120, 40);
    for (index, line) in lines.iter().enumerate() {
        assert!(
            !line.contains('\n') && !line.contains('\r'),
            "第 {index} 行含内嵌换行：{line:?}"
        );
    }
}

/// 多行状态在屏幕上应当是**多行**（不是一行被截断），且三行都能看到内容。
#[test]
fn status_with_newlines_renders_every_line() {
    let mut state = base_state();
    state
        .transcript
        .push(TranscriptItem::Status(TODO_LIST.to_string()));
    let lines = render_all(&state, 120, 40);
    let rendered: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("#1 1a") || line.contains("#2 1b") || line.trim() == "[ ]")
        .collect();
    assert!(
        rendered.len() >= 3,
        "三行清单应各占一行，实际命中 {} 行：{lines:#?}",
        rendered.len()
    );
}

/// 多行 `Notice`（`showWarning` 也会收到多行文本）同理。
#[test]
fn notice_with_newlines_never_leaks_embedded_newlines() {
    let mut state = base_state();
    state
        .transcript
        .push(TranscriptItem::Notice(format!("Warning: {TODO_LIST}")));
    let lines = render_all(&state, 120, 40);
    for (index, line) in lines.iter().enumerate() {
        assert!(
            !line.contains('\n') && !line.contains('\r'),
            "第 {index} 行含内嵌换行：{line:?}"
        );
    }
}

/// **核心回归**：这条多行状态不得让 `diff_render` 返回错误。
///
/// 改前这里是 `Err(LineTooWide { index: .., width: 120, line_width: 122 })`，
/// 而 TUI 主循环把该错误当成"这一帧不画"⇒ 后续每一帧都失败 ⇒ 屏幕永久冻结。
#[test]
fn diff_render_accepts_multiline_status() {
    // 必须是**两帧**：首帧走 `full_render_styled`（不校验行宽），
    // 第二帧才走增量路径 —— 那才是真机上真正报 `LineTooWide` 的地方。
    let mut state = base_state();
    state
        .transcript
        .push(TranscriptItem::Assistant("前一段正文。".to_string()));
    let mut cache = RenderCache::new();
    let mut diff = DiffState::new();
    let first = render_all_marker_cached_styled(&state, 120, 40, &mut cache);
    let _ = diff_render_with_cursor_styled(
        &mut diff,
        &first.lines,
        &first.styles,
        &first.spans,
        120,
        40,
    )
    .expect("首帧应能渲染");

    // 扩展的 `ctx.ui.notify`（多行清单）此时到达。
    state
        .transcript
        .push(TranscriptItem::Status(TODO_LIST.to_string()));
    let second = render_all_marker_cached_styled(&state, 120, 40, &mut cache);
    let result = diff_render_with_cursor_styled(
        &mut diff,
        &second.lines,
        &second.styles,
        &second.spans,
        120,
        40,
    );
    assert!(
        result.is_ok(),
        "多行状态让增量帧渲染失败（改前即 LineTooWide）：{:?}",
        result.err()
    );
}

/// **真机症状的直接断言**：改完内容后第二帧必须真的写出字节。
///
/// 冻结时每帧都是 0 字节——只断言"没有报错"不够，必须断言"有输出"。
#[test]
fn second_frame_after_multiline_status_is_not_empty() {
    let mut state = base_state();
    state
        .transcript
        .push(TranscriptItem::Status(TODO_LIST.to_string()));
    let mut cache = RenderCache::new();
    let mut diff = DiffState::new();
    let first = render_all_marker_cached_styled(&state, 120, 40, &mut cache);
    let (first_bytes, _cursor) = diff_render_with_cursor_styled(
        &mut diff,
        &first.lines,
        &first.styles,
        &first.spans,
        120,
        40,
    )
    .expect("首帧应能渲染");
    assert!(!first_bytes.is_empty(), "首帧必须写出内容");

    // 第二帧：只改编辑器输入（真机上"按了键却没反应"就是这一步没有输出）。
    state.input = "x".to_string();
    state.cursor = 1;
    let second = render_all_marker_cached_styled(&state, 120, 40, &mut cache);
    let (second_bytes, _cursor2) = diff_render_with_cursor_styled(
        &mut diff,
        &second.lines,
        &second.styles,
        &second.spans,
        120,
        40,
    )
    .expect("第二帧应能渲染");
    assert!(
        !second_bytes.is_empty(),
        "改了输入却写出 0 字节 ⇒ 用户看到的就是'按键无反应'"
    );
}
