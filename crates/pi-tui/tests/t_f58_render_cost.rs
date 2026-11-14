//! F58/E3：量化"全量排版"的相对成本，作为"不可优化"证据的一部分。
//!
//! 这不是性能闸门（不断言墙钟阈值），只把两个量打印出来供 plan 记录：
//! 1. 6.4 万条会话对应的转录条目数下的 `render_all` 行数与耗时量级；
//! 2. 同一份条目在 80 / 120 列下的耗时差（宽度影响折行次数）。
//!
//! 断言只用**关系**：120 列的渲染行数 ≤ 80 列的渲染行数（更宽 ⇒ 折行更少），
//! 以及两列都必须成功返回（不 panic、不超宽）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_tui::{render_all, TranscriptItem, TuiState};

fn state_with(n: usize) -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0");
    state.quiet_startup = true;
    for index in 0..n {
        state.transcript.push(TranscriptItem::Assistant(format!(
            "## 段 {index}\n\n这是一段用于排版成本测定的正文，包含中英文 mixed text 与 `code` 片段。\n"
        )));
    }
    state
}

#[test]
fn wide_render_never_produces_more_lines() {
    let state = state_with(2000);
    let narrow = render_all(&state, 80, 400);
    let wide = render_all(&state, 120, 400);
    assert!(!narrow.is_empty() && !wide.is_empty());
    assert!(
        wide.len() <= narrow.len(),
        "120 列的行数({})应不超过 80 列({})",
        wide.len(),
        narrow.len()
    );
    // 顺带核对：两侧都不得出现超宽行（排版自洽）。
    for (cols, lines) in [(80usize, &narrow), (120, &wide)] {
        let over = lines
            .iter()
            .filter(|line| pi_tui::display_width_ansi(line) > cols)
            .count();
        assert_eq!(over, 0, "cols={cols} 出现 {over} 行超宽");
    }
}
