//! F51/Z2.3：**宽度不变量** —— 任何渲染行在任一宽度下都不得超出终端列数。
//!
//! 用户真机现象：`-c` 恢复后工具输出「超出屏宽的部分不折行、伸出终端之外」。
//! ref 的模型是「组件 `render(width)` 必须自己保证每行 ≤ width」（`tui-main-screen.ts`
//! 的 `fullRender` 只按 `\r\n` 分行、不做二次折行），越界会让终端自行折行、把画面挤乱。
use pi_tui::theme::{Palette, Theme};
use pi_tui::{render_all, TranscriptItem, TuiState};

fn state_with(items: Vec<TranscriptItem>) -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.transcript = items;
    state
}

/// 去掉 SGR/OSC 之后的可见宽度（`display_width` 把转义序列当普通字符数）。
fn visible(text: &str) -> usize {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == 0x1b {
            index += 1;
            if index < bytes.len() && bytes[index] == b'[' {
                index += 1;
                while index < bytes.len() && !(0x40..=0x7e).contains(&bytes[index]) {
                    index += 1;
                }
                if index < bytes.len() {
                    index += 1;
                }
            } else if index < bytes.len() && bytes[index] == b']' {
                while index < bytes.len() && bytes[index] != 0x07 {
                    index += 1;
                }
                if index < bytes.len() {
                    index += 1;
                }
            } else if index < bytes.len() {
                index += 1;
            }
            continue;
        }
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        out.push(ch);
        index += ch.len_utf8();
    }
    pi_tui::display_width(&out)
}

fn assert_fits(name: &str, state: &TuiState, cols: usize) {
    let lines = render_all(state, cols, 40);
    for (index, line) in lines.iter().enumerate() {
        let width = visible(line);
        assert!(
            width <= cols,
            "{name}: 第 {index} 行宽度 {width} 超过 {cols} 列：{line:?}"
        );
    }
}

/// 长英文行（无空格，无法按词折行）。
#[test]
fn long_ascii_tool_output_fits() {
    let long = "x".repeat(400);
    let state = state_with(vec![TranscriptItem::ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        summary: "run".into(),
        args_json: None,
        tool_known: Some(true),
        output: Some(format!("{long}\n{long}")),
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        collapsed_row: None,
    }]);
    for cols in [80, 110] {
        assert_fits("long-ascii", &state, cols);
    }
}

/// CJK 长行（宽字符，双列）。
#[test]
fn long_cjk_tool_output_fits() {
    let long = "中文内容测试".repeat(40);
    let state = state_with(vec![TranscriptItem::ToolCall {
        id: "c2".into(),
        name: "bash".into(),
        summary: "run".into(),
        args_json: None,
        tool_known: Some(true),
        output: Some(long),
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        collapsed_row: None,
    }]);
    for cols in [80, 110] {
        assert_fits("long-cjk", &state, cols);
    }
}

/// 含 ANSI 的扩展行（`clamp_to_width_ansi` 路径）。
#[test]
fn long_ansi_extension_line_fits() {
    let painted = format!("\u{1b}[38;5;109m{}\u{1b}[0m", "y".repeat(300));
    let state = state_with(vec![TranscriptItem::ToolCall {
        id: "c3".into(),
        name: "zeta".into(),
        summary: "run".into(),
        args_json: None,
        tool_known: Some(true),
        output: None,
        is_error: false,
        started_at: None,
        took: None,
        render_lines: Some(vec![painted]),
        result_render_lines: None,
        result_payload: None,
        collapsed_row: None,
    }]);
    for cols in [80, 110] {
        assert_fits("long-ansi", &state, cols);
    }
}

/// 折叠行（扩展原型补丁返回）本身也可能是长行。
#[test]
fn long_collapsed_row_fits() {
    let state = state_with(vec![TranscriptItem::ToolCall {
        id: "c4".into(),
        name: "zeta".into(),
        summary: "run".into(),
        args_json: None,
        tool_known: Some(true),
        output: Some("done".into()),
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        collapsed_row: Some(vec![format!("  + zeta · {} · ok", "z".repeat(300))]),
    }]);
    for cols in [80, 110] {
        assert_fits("long-collapsed", &state, cols);
    }
    // 顺带确认调色板可用（避免测试因无关原因失败）。
    let _ = Palette::of(&Theme::builtin("dark"));
}
