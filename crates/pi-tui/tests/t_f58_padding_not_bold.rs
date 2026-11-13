//! F58/B：**补位空格只带底色，不带 bold/dim/italic/underline/strike**。
//!
//! ref 的分支（`packages/tui/src/components/text.ts:74-84`）：
//! - 有 `customBgFn` ⇒ `applyBackgroundToLine(line + padding)`（`utils.ts:1050-1058`），
//!   而 `bgFn` 只做 `theme.bg(...)`，**不会**重复施加 bold；
//! - 无背景 ⇒ `line + " ".repeat(n)`，不带任何 SGR。
//!
//! 另外 `wrapTextWithAnsi` 对每个输出行 `trimEnd()` 并在非末行补 `lineEndReset`
//! （`utils.ts:920-935`），折行断点的属性也不会延续到 padding。
//!
//! 改前（F54 引入的过度泛化）：补位空格继承行尾字符的 bold，导致真实会话 110 列的
//! `report_to_user` 工具行整行 109 格加粗（TS 只有工具名 14 格），单行差 95 格。
//! 这就是用户截图里"pi-rs 有粗体、TS 无"的来源。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_tui::diff::{diff_render_with_cursor_styled, DiffState};
use pi_tui::style::{LineStyle, Span};

/// 渲染一行，返回 `(整行字节, 该行内每一列的 bold 标记)`。
fn render_one_line(width: usize, spans: &[Span], text: &str) -> (String, Vec<bool>) {
    let lines = vec![text.to_string()];
    let styles = vec![None];
    let mut state = DiffState::new();
    let (bytes, _cursor) =
        diff_render_with_cursor_styled(&mut state, &lines, &styles, &[spans.to_vec()], width, 4)
            .expect("渲染应成功");
    // 用最小终端模型统计每列 bold：逐字符走，SGR 只跟踪 1/22/0。
    let mut bold_cols: Vec<bool> = Vec::new();
    let mut bold = false;
    let mut chars = bytes.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                let mut params = String::new();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        if next == 'm' {
                            for part in params.split(';') {
                                match part {
                                    "" | "0" => bold = false,
                                    "1" => bold = true,
                                    "22" => bold = false,
                                    _other => {}
                                }
                            }
                        }
                        break;
                    }
                    params.push(next);
                }
            }
            continue;
        }
        if ch == '\r' || ch == '\n' {
            continue;
        }
        bold_cols.push(bold);
    }
    (bytes, bold_cols)
}

/// 有底色的行：补位空格**不得**带 bold（改前会带，实测整行都是粗体）。
#[test]
fn padding_after_bold_span_is_not_bold() {
    // 复刻 `report_to_user` 工具行：1 列 padding + 粗体工具名 + 补位空格。
    let name = "report_to_user";
    let text = format!(" {name}{}", " ".repeat(30));
    let spans = [Span::new(
        1,
        1 + name.len(),
        LineStyle::fg(pi_tui::style::TEXT)
            .with_bg(pi_tui::style::TOOL_SUCCESS_BG)
            .bold(),
    )];
    let (_bytes, bold) = render_one_line(60, &spans, &text);
    // 只有工具名那段（列 1..1+len）是粗体。
    let bold_cols: Vec<usize> = bold
        .iter()
        .enumerate()
        .filter(|(_, flag)| **flag)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        bold_cols.first().copied(),
        Some(1),
        "粗体应从第 1 列（工具名首字符）开始"
    );
    assert_eq!(
        bold_cols.last().copied(),
        Some(name.len()),
        "粗体必须在工具名末尾结束（补位空格不得加粗）：{bold_cols:?}"
    );
}

/// 补位空格的数量足够多时（truecolor 背景），仍不得把 bold 带过去。
#[test]
fn long_padding_never_inherits_attributes() {
    let text = format!(" {}{}", "x", " ".repeat(100));
    let spans = [Span::new(
        1,
        2,
        LineStyle::fg(pi_tui::style::TEXT)
            .with_bg(pi_tui::style::TOOL_SUCCESS_BG)
            .bold(),
    )];
    let (_bytes, bold) = render_one_line(120, &spans, &text);
    let bold_count = bold.iter().filter(|flag| **flag).count();
    assert_eq!(bold_count, 1, "只有那 1 个粗体字符：{bold_count}");
}
