//! F52/A+B：`ctrl+o` 双向切换与 `ctrl+t` 实效。
//!
//! ref：
//! - `interactive-mode.ts:4397-4416` `toggleToolOutputExpansion` →
//!   `setToolsExpanded(!toolOutputExpanded)` → 对每个可展开子组件 `setExpanded()`；
//! - `tool-execution.ts:244-247` `setExpanded()` 只改字段 + `updateDisplay()`，
//!   补丁在 `expanded === true` 时回落原生（`pi-ai-summary/.../tool-render.ts:186-196`）；
//! - `interactive-mode.ts:4429-4434` `toggleThinkingBlockVisibility`；
//!   `assistant-message.ts:143-145` 隐藏时渲染 `hiddenThinkingLabel`（默认 `"Thinking..."`），
//!   可见时渲染思考全文。
use pi_tui::theme::{Palette, Theme};
use pi_tui::{render_all, TranscriptItem, TuiState};

fn palette() -> Palette {
    let _ = Palette::of(&Theme::builtin("dark"));
    Palette::of(&Theme::builtin("dark"))
}

fn tool_state() -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.transcript.push(TranscriptItem::ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        summary: "run".into(),
        args_json: None,
        tool_known: Some(true),
        output: Some("FULL-OUTPUT-LINE-1\nFULL-OUTPUT-LINE-2".into()),
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        collapsed_row: Some(vec!["  + bash · run · ok".to_string()]),
    });
    let _ = palette();
    state
}

fn join(state: &TuiState, cols: usize) -> String {
    render_all(state, cols, 30).join("\n")
}

/// 折叠 → 展开 → 再折叠：两个方向都必须生效，且不丢任一态。
#[test]
fn ctrl_o_toggles_both_ways() {
    let mut state = tool_state();
    let collapsed = join(&state, 110);
    assert!(
        collapsed.contains("+ bash · run · ok"),
        "折叠态应显示折叠行：{collapsed}"
    );
    assert!(
        !collapsed.contains("FULL-OUTPUT-LINE-1"),
        "折叠态不应显示全文"
    );

    state.tools_expanded = true;
    let expanded = join(&state, 110);
    assert!(
        !expanded.contains("+ bash · run · ok"),
        "展开态**不得**再显示折叠行（真机 E3）：{expanded}"
    );
    assert!(
        expanded.contains("FULL-OUTPUT-LINE-1"),
        "展开态应显示全文：{expanded}"
    );

    state.tools_expanded = false;
    let back = join(&state, 110);
    assert!(
        back.contains("+ bash · run · ok"),
        "再按 ctrl+o 应回到折叠行：{back}"
    );
}

/// `ctrl+t`：隐藏态渲染标签，可见态渲染**全文**。
#[test]
fn ctrl_t_switches_between_label_and_full_text() {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.transcript.push(TranscriptItem::Thinking {
        text: "FULL-THINKING-BODY".into(),
        label: Some("+ Thought: preview".into()),
    });

    state.hide_thinking_block = true;
    let hidden = join(&state, 110);
    assert!(
        hidden.contains("+ Thought: preview"),
        "隐藏态应显示扩展标签：{hidden}"
    );
    assert!(
        !hidden.contains("FULL-THINKING-BODY"),
        "隐藏态不得显示全文：{hidden}"
    );
    assert!(
        !hidden.contains("- Thought"),
        "标签不得被 markdown 当成列表符号渲染成 `- `（真机 E4）：{hidden}"
    );

    state.hide_thinking_block = false;
    let visible = join(&state, 110);
    assert!(
        visible.contains("FULL-THINKING-BODY"),
        "可见态必须渲染思考全文（真机 E4「无变化」）：{visible}"
    );
}
