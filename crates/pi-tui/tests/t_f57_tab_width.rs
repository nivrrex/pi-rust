//! F57/D3：扩展渲染行里的 `\t` 必须先展开成 3 空格再折行。
//!
//! ref `packages/tui/src/components/text.ts:61`：`Text.render` 在排版前做
//! `this.text.replace(/\t/g, "   ")`。`unicode-width` 把 `\t` 算成 **0 列**，
//! 所以不展开时折行器以为整行很窄，而真实终端按制表位展开 ⇒ 渲染行远超 `cols`，
//! `diff_render` 报 `rendered line N exceeds terminal width`（D2 的 `121 > 120` 同源）。
//!
//! 改前实测：`cols=80`、`render_lines = Some(vec!["\t".repeat(100)])` ⇒ 渲染出 **101 列**的行。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_tui::{render_all, TranscriptItem, TuiState};

fn state_with_ext_lines(lines: Vec<String>) -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0");
    state.quiet_startup = true;
    state.transcript.push(TranscriptItem::ToolCall {
        id: "call-tab".to_string(),
        name: "ext_probe".to_string(),
        summary: "x".to_string(),
        args_json: None,
        output: None,
        is_error: false,
        started_at: None,
        took: None,
        render_lines: Some(lines),
        result_render_lines: None,
        result_payload: None,
        tool_known: Some(true),
        collapsed_row: None,
    });
    state
}

/// 每一行的**可见宽度**都不得超过 `cols`（改前：`\t` 未展开 ⇒ 101 > 80）。
#[test]
fn extension_render_tabs_never_exceed_width() {
    for cols in [80usize, 120] {
        let state = state_with_ext_lines(vec!["\t".repeat(100)]);
        let over: Vec<(usize, usize)> = render_all(&state, cols, 400)
            .iter()
            .enumerate()
            .map(|(index, line)| (index, pi_tui::display_width_ansi(line)))
            .filter(|(_, width)| *width > cols)
            .collect();
        assert!(
            over.is_empty(),
            "cols={cols} 时有 {} 行超宽（行号,宽度）={:?}",
            over.len(),
            &over[..over.len().min(5)]
        );
    }
}

/// 制表符本身不得出现在渲染结果里（必须已展开成空格）。
#[test]
fn extension_render_has_no_raw_tabs() {
    let state = state_with_ext_lines(vec!["a\tb\tc".to_string()]);
    for line in render_all(&state, 80, 400) {
        assert!(!line.contains('\t'), "渲染行里不应残留制表符：{line:?}");
    }
}

/// 结果渲染行（`result_render_lines`）同样要展开。
#[test]
fn result_render_tabs_never_exceed_width() {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0");
    state.quiet_startup = true;
    state.transcript.push(TranscriptItem::ToolCall {
        id: "call-tab-result".to_string(),
        name: "ext_probe".to_string(),
        summary: "x".to_string(),
        args_json: None,
        output: None,
        is_error: false,
        started_at: None,
        took: None,
        render_lines: Some(vec!["head".to_string()]),
        result_render_lines: Some(vec!["\t".repeat(100)]),
        result_payload: None,
        tool_known: Some(true),
        collapsed_row: None,
    });
    let over: Vec<usize> = render_all(&state, 80, 400)
        .iter()
        .map(|line| pi_tui::display_width_ansi(line))
        .filter(|width| *width > 80)
        .collect();
    assert!(over.is_empty(), "结果渲染行超宽：{over:?}");
}
