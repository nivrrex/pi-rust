//! F53e：段落空行/折行断点的回归用例。
//!
//! 三个用例分别锁定真实会话暴露过的三类"差 1–2 行"：
//! 1. 命中每条目缓存时也必须走同一份段落空行判定；
//! 2. `Status` 自带 `Spacer(1)`（上一条是工具块时要自己补）；
//! 3. CJK 断点集合按 `Script_Extensions` 的实际枚举结果（`、`/`。` 是断点，`，`/`（` 不是）。

use pi_tui::{render_all, render_all_marker_cached, RenderCache, TranscriptItem, TuiState};

fn base_state() -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.quiet_startup = true;
    state
}

fn tool(state: &mut TuiState, name: &str, output: &str) {
    state.transcript.push(TranscriptItem::ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        summary: "x.txt".to_string(),
        args_json: Some(r#"{"path":"x.txt"}"#.to_string()),
        output: Some(output.to_string()),
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        tool_known: Some(false),
        collapsed_row: None,
    });
}

/// 缓存命中的第二帧必须与首帧、与不走缓存的 `render_all` **逐行相同**。
///
/// 改前：快路径无条件补段落空行，`Status` 与"思考块紧跟助手"两类各多 1 行。
#[test]
fn cached_frames_match_uncached_on_blank_lines() {
    let mut state = base_state();
    state.hide_thinking_block = true;
    // **连续两条助手消息**：第二条的段落空行要由第一条的尾随空行顶替
    // （`paragraph_blank` 的 `follows_assistant` 分支）。这条路径的条目**会进缓存**
    // （隐藏态思考块在渲染时 `continue`，不写缓存，因此那种组合测不出快路径）。
    state
        .transcript
        .push(TranscriptItem::Assistant("第一段。".into()));
    state
        .transcript
        .push(TranscriptItem::Assistant("第二段。".into()));
    tool(&mut state, "read", "内容\n");
    state
        .transcript
        .push(TranscriptItem::Status("Tool output: expanded".into()));

    let plain = render_all(&state, 100, 60);
    let mut cache = RenderCache::new();
    let first = render_all_marker_cached(&state, 100, 60, &mut cache);
    let second = render_all_marker_cached(&state, 100, 60, &mut cache);
    assert_eq!(first, second, "缓存命中后帧内容必须一致");
    // 带 marker 的入口会把光标画成反显空格，去掉它再与 `render_all` 比。
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
}

/// `Status` 跟在工具块后面时，工具块的尾随空行是 `Box` 底 padding，
/// 不能顶替 ref `showStatus` 自己的 `Spacer(1)` —— 必须有两行空行。
#[test]
fn status_after_tool_keeps_its_own_spacer() {
    let mut state = base_state();
    tool(&mut state, "read", "内容\n");
    state
        .transcript
        .push(TranscriptItem::Status("Tool output: expanded".into()));
    let lines = render_all(&state, 100, 60);
    let index = lines
        .iter()
        .position(|line| line.contains("Tool output: expanded"))
        .expect("状态行应在帧里");
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
    assert_eq!(
        blanks, 2,
        "工具块之后的状态行前应有 2 行空行（底 padding + Spacer）"
    );
}

/// `、`（U+3001）与 `。`（U+3002）的 `Script_Extensions` 含 Han ⇒ 在断点集合里；
/// `，`（U+FF0C）与 `（`（U+FF08）只有 Common ⇒ 不在。
///
/// 改前按"块范围"近似，漏掉 0x3001..0x303F ⇒ `id=27721、` 被当成一个不可分 token，
/// 整段中文提前 8 列换行（真实会话 TS 107 列 / 我们 109 列）。
#[test]
fn cjk_breakpoints_follow_script_extensions() {
    let text = "再完善文档中第24期目录的条目描述（补充邮件id=27721、修正 ocr/ 内为 12 张图的分片 txt/png）";
    let lines = pi_tui::wrap_to_width(text, 50);
    assert!(
        lines.iter().any(|line| line.starts_with('、')),
        "`、` 必须自成 token、可以出现在续行行首：{lines:?}"
    );
    // `（` 不在断点集合里 ⇒ 它不会单独占一行开头。
    assert!(
        !lines.iter().any(|line| line.starts_with('（')),
        "`（` 不是断点：{lines:?}"
    );
}
