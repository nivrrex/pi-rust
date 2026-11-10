//! F53b/H：扩展 `renderCall` 返回的行超宽时**必须折行**，不得截断。
//!
//! ref 把渲染器返回的组件放进 `tool-execution.ts:98-108` 的 `Box(1, 1, …)`，
//! 组件是 `Text`（`text.ts:60-68`：`replace(/\t/g,"   ")` → `wrapTextWithAnsi(...)`），
//! 因此超宽行是**折行**。此前 pi-rs 对扩展**调用行**用 `clamp_to_width_ansi` 截断，
//! 真机 H 里 `(timeout 600s)` 就是这样丢掉的。
use pi_tui::{display_width_ansi, render_all, strip_ansi, TranscriptItem, TuiState};

fn state_with_call_line(line: String) -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.transcript.push(TranscriptItem::ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        summary: "echo hi".into(),
        args_json: None,
        tool_known: Some(true),
        output: None,
        is_error: false,
        started_at: None,
        took: None,
        render_lines: Some(vec![line]),
        result_render_lines: None,
        result_payload: None,
        collapsed_row: None,
    });
    state
}

/// 扩展头行的尾部必须保留（折行），且每行不超过终端宽度。
#[test]
fn extension_call_line_wraps_instead_of_truncating() {
    const HEIGHT: usize = 60;
    let tail = "TAIL-MARKER-(timeout 600s)";
    let long = format!("{}{tail}", "宽字符内容".repeat(20));
    // 带 SGR，模拟扩展用 `theme.fg(...)` 生成的行。
    let line = format!("\u{1b}[31m{long}\u{1b}[0m");
    let state = state_with_call_line(line);
    let frame = render_all(&state, 80, HEIGHT);
    let plain = strip_ansi(&frame.join("\n"));
    assert!(
        plain.contains(tail),
        "扩展头行的尾部被截断（应为折行）：\n{plain}"
    );
    for (index, line) in frame.iter().enumerate() {
        let width = display_width_ansi(line);
        assert!(width <= 80, "第 {index} 行宽 {width} 超 80：{line:?}");
    }
}
