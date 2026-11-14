//! F52 债务②：`Status` 行的**前导空行**与**缓存基准**必须同时正确。
//!
//! 两件事看起来独立、实际耦合：
//! 1. ref `showStatus`（`interactive-mode.ts:3695-3711`）= `Spacer(1)` + `Text(msg,1,0)`，
//!    所以状态行前面**恰好 1 行空行**、后面不空；
//! 2. 条目渲染缓存以 `content_start`（内容起点）为基准写回。若把"段落空行"也算进内容，
//!    或反过来在补完空行后又 `pop()` 掉，缓存里的行号会与下次渲染错位 ——
//!    表现为"第一帧有状态行、第二帧消失"（本项目实际踩过）。
//!
//! 本用例把两条一起钉住：缓存命中前后帧必须逐行相同，且状态行前恰好 1 行空行。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_tui::{render_all, render_all_marker_cached, RenderCache, TranscriptItem, TuiState};

fn base_state() -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.quiet_startup = true;
    state
}

fn status_blank_lines(lines: &[String], needle: &str) -> usize {
    let index = lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("帧里应出现 {needle:?}"));
    let mut blanks = 0usize;
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if lines[cursor].trim().is_empty() {
            blanks += 1;
        } else {
            break;
        }
    }
    blanks
}

/// 助手正文之后的状态行：前导空行由助手的尾随空行提供 ⇒ **恰好 1 行**。
#[test]
fn status_after_assistant_has_one_leading_blank() {
    let mut state = base_state();
    state
        .transcript
        .push(TranscriptItem::Assistant("前一段正文。".to_string()));
    state
        .transcript
        .push(TranscriptItem::Status("Thinking level: high".to_string()));
    let lines = render_all(&state, 100, 60);
    assert_eq!(
        status_blank_lines(&lines, "Thinking level: high"),
        1,
        "助手之后的扩展状态行前应有 1 行空行"
    );
}

/// 工具块之后的状态行：工具块的尾随空行是 `Box` 底 padding，不能顶替 `Spacer(1)`
/// ⇒ **恰好 2 行**。这条锁住 F53e 的修正（此前被当成 1 行，整帧少 1 行）。
#[test]
fn status_after_tool_has_two_leading_blanks() {
    let mut state = base_state();
    state.transcript.push(TranscriptItem::ToolCall {
        id: "call-1".to_string(),
        name: "read".to_string(),
        summary: "x.txt".to_string(),
        args_json: Some(r#"{"path":"x.txt"}"#.to_string()),
        output: Some("内容\n".to_string()),
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        tool_known: Some(false),
        collapsed_row: None,
    });
    state
        .transcript
        .push(TranscriptItem::Status("Tool output: expanded".to_string()));
    let lines = render_all(&state, 100, 60);
    assert_eq!(
        status_blank_lines(&lines, "Tool output: expanded"),
        2,
        "工具块之后的状态行前应有 2 行空行（底 padding + Spacer）"
    );
}

/// **缓存基准**：`Status` 参与条目缓存时，第二帧（命中缓存）必须逐行等于第一帧。
///
/// 这是"状态行第一帧有、第二帧消失"那个真实缺陷的回归：缓存写回若以
/// `before + 1` 而非 `content_start` 为基准，`Status`（不补段落空行）会整行漏进缓存
/// 边界之外，第二帧就不再输出它。
#[test]
fn status_survives_cache_hit_unchanged() {
    let mut state = base_state();
    state
        .transcript
        .push(TranscriptItem::Assistant("正文。".to_string()));
    state
        .transcript
        .push(TranscriptItem::Status("Thinking level: high".to_string()));
    state
        .transcript
        .push(TranscriptItem::Assistant("再一段。".to_string()));

    let plain = render_all(&state, 100, 60);
    let mut cache = RenderCache::new();
    let first = render_all_marker_cached(&state, 100, 60, &mut cache);
    let second = render_all_marker_cached(&state, 100, 60, &mut cache);
    assert_eq!(first, second, "缓存命中后帧必须与首帧逐行相同");
    // 带 marker 的入口会把光标换成一个反显空格，剥掉后再与不缓存路径比。
    let strip = |lines: Vec<String>| -> Vec<String> {
        lines
            .into_iter()
            .map(|line| {
                line.replace(pi_tui::diff::CURSOR_MARKER, "")
                    .replace("\u{1b}[7m \u{1b}[0m", "")
            })
            .collect()
    };
    assert_eq!(strip(first), plain, "缓存路径与不缓存路径必须逐行相同");
    assert!(
        second
            .iter()
            .any(|line| line.contains("Thinking level: high")),
        "第二帧仍应包含状态行（缓存基准错位会让它消失）"
    );
}
