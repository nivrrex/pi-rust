//! F60/12：树导航后会话区必须**整段替换**（ref `interactive-mode.ts:1952-1957`）。
//!
//! ref 在 `navigateTree` 之后做 `chatContainer.clear()` + `renderInitialMessages()`
//! ——按新分支重渲染，而不是在旧内容后面追加一块。我们此前只追加 `[branch]` 块
//! ⇒ 换分支后屏幕上仍是旧分支的全部内容（越用越长）。
//!
//! 本用例钉住 `TuiEvent::TreeNavigated` 的语义：给定新分支的会话区，
//! 旧内容**全部消失**，且摘要（若给）排在末尾。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_tui::{apply, render_all, TranscriptItem, TuiAction, TuiEvent, TuiState};

fn base_state() -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.quiet_startup = true;
    state
}

/// 旧分支的内容必须被**清掉**（不是追加）。
#[test]
fn tree_navigation_replaces_the_transcript() {
    let mut state = base_state();
    state
        .transcript
        .push(TranscriptItem::User("旧分支的问题".to_string()));
    state
        .transcript
        .push(TranscriptItem::Assistant("旧分支的回答".to_string()));

    let new_transcript = vec![
        TranscriptItem::User("新分支的问题".to_string()),
        TranscriptItem::Assistant("新分支的回答".to_string()),
    ];
    assert_eq!(
        apply(
            &mut state,
            TuiEvent::TreeNavigated {
                transcript: new_transcript,
                summary: None,
            }
        ),
        TuiAction::Redraw
    );
    assert_eq!(state.transcript.len(), 2, "会话区应被整段替换");
    let lines = render_all(&state, 100, 40).join("\n");
    assert!(
        !lines.contains("旧分支"),
        "旧分支内容必须消失（ref 会 clear）。屏幕：\n{lines}"
    );
    assert!(
        lines.contains("新分支的问题") && lines.contains("新分支的回答"),
        "新分支内容应出现。屏幕：\n{lines}"
    );
}

/// 有摘要时，`[branch]` 块排在**末尾**（ref 在 `renderInitialMessages` 之后再加）。
#[test]
fn tree_navigation_appends_the_summary_block_last() {
    let mut state = base_state();
    state
        .transcript
        .push(TranscriptItem::User("旧".to_string()));
    let new_transcript = vec![TranscriptItem::User("新".to_string())];
    let _ = apply(
        &mut state,
        TuiEvent::TreeNavigated {
            transcript: new_transcript,
            summary: Some("分支摘要正文".to_string()),
        },
    );
    assert_eq!(state.transcript.len(), 2);
    assert!(
        matches!(
            state.transcript.last(),
            Some(TranscriptItem::BranchSummary { .. })
        ),
        "摘要块应在末尾，实际：{:?}",
        state.transcript.last()
    );
    // 折叠态只显示提示行（ref `branch-summary-message.ts:32-53`：折叠时正文不可见），
    // 展开态才渲染 markdown 正文（`ctrl+o` 切换）。
    let collapsed = render_all(&state, 100, 40).join("\n");
    assert!(
        collapsed.contains("Branch summary (ctrl+o to expand)"),
        "折叠态应显示展开提示。屏幕：\n{collapsed}"
    );
    assert!(
        !collapsed.contains("分支摘要正文"),
        "折叠态不应显示摘要正文（ref 只在展开时渲染）。屏幕：\n{collapsed}"
    );
    state.tools_expanded = true;
    let expanded = render_all(&state, 100, 40).join("\n");
    assert!(
        expanded.contains("分支摘要正文"),
        "展开态应显示摘要正文。屏幕：\n{expanded}"
    );
}
