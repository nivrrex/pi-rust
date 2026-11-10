//! F52（F51 债务）：渲染缓存键必须覆盖**本批新增的两种状态**，否则回填/切换后不重绘。
//!
//! 这两条都是真机/对拍里实际踩过的：
//! - `collapsed_row` 不进键 ⇒ 扩展回填折叠行后键不变，每帧命中旧缓存，折叠行永不出现
//!   （F51/Z1 最难的 bug，真实 95 MB 会话整屏重绘 80 次）；
//! - `hide_thinking_block` 与标签不进键 ⇒ `ctrl+t` 只改状态不重绘（F52/B）。
use pi_tui::{render_all_marker_cached_styled, RenderCache, TranscriptItem, TuiState};

fn state_with_tool() -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.transcript.push(TranscriptItem::ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        summary: "run".into(),
        args_json: None,
        tool_known: Some(true),
        output: Some("FULL-OUTPUT-LINE-1".into()),
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        collapsed_row: None,
    });
    state
}

fn frame(state: &TuiState, cache: &mut RenderCache) -> String {
    render_all_marker_cached_styled(state, 110, 40, cache)
        .lines
        .join("\n")
}

/// 折叠行回填后必须**重新渲染**（键里含 `collapsed_row`）。
#[test]
fn collapsed_row_change_invalidates_cache_entry() {
    let mut state = state_with_tool();
    let mut cache = RenderCache::new();
    let before = frame(&state, &mut cache);
    assert!(!before.contains("+ bash · run · ok"), "回填前不应有折叠行");

    if let Some(TranscriptItem::ToolCall { collapsed_row, .. }) = state.transcript.first_mut() {
        *collapsed_row = Some(vec!["  + bash · run · ok".to_string()]);
    }
    let after = frame(&state, &mut cache);
    assert!(
        after.contains("+ bash · run · ok"),
        "回填折叠行后必须重绘，实际：\n{after}"
    );
    assert!(
        !after.contains("FULL-OUTPUT-LINE-1"),
        "折叠态应替代原生块正文：\n{after}"
    );
}

/// `ctrl+t`（`hide_thinking_block` 切换）必须让思考块那一行重绘。
#[test]
fn hide_thinking_block_toggle_invalidates_cache_entry() {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.transcript.push(TranscriptItem::Thinking {
        text: "THINKING-BODY-LINE".into(),
        label: Some("+ Thought: 合成预览".into()),
    });
    let mut cache = RenderCache::new();
    state.hide_thinking_block = true;
    let hidden = frame(&state, &mut cache);
    assert!(hidden.contains("+ Thought: 合成预览"), "{hidden}");
    assert!(!hidden.contains("THINKING-BODY-LINE"), "{hidden}");

    state.hide_thinking_block = false;
    let visible = frame(&state, &mut cache);
    assert!(
        visible.contains("THINKING-BODY-LINE"),
        "切到可见态必须重绘出全文：{visible}"
    );

    state.hide_thinking_block = true;
    let again = frame(&state, &mut cache);
    assert!(again.contains("+ Thought: 合成预览"), "{again}");
    assert!(!again.contains("THINKING-BODY-LINE"), "{again}");
}

/// 标签内容变化（扩展改了 `setHiddenThinkingLabel`）也必须重绘。
#[test]
fn thinking_label_change_invalidates_cache_entry() {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.hide_thinking_block = true;
    state.transcript.push(TranscriptItem::Thinking {
        text: "body".into(),
        label: Some("+ Thought: 甲".into()),
    });
    let mut cache = RenderCache::new();
    let first = frame(&state, &mut cache);
    assert!(first.contains("+ Thought: 甲"), "{first}");
    if let Some(TranscriptItem::Thinking { label, .. }) = state.transcript.first_mut() {
        *label = Some("+ Thought: 乙".into());
    }
    let second = frame(&state, &mut cache);
    assert!(second.contains("+ Thought: 乙"), "{second}");
    assert!(!second.contains("+ Thought: 甲"), "{second}");
}
