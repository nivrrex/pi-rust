//! P9-F28/S3：`[Extension issues]` 按宽度**折行**（ref `interactive-mode.ts:1874-1894`
//! 的 `Text` 组件语义），不是截断。

use pi_tui::{render, TuiState};

fn frame_with_issue(cols: usize) -> Vec<String> {
    let mut state = TuiState::new("0.0.0", "/tmp", "mock-model");
    state
        .extension_issues
        .push("扩展宿主启动失败：这是一个很长的错误信息用于验证折行行为，不应被截断".to_string());
    render(&state, cols, 40)
}

#[test]
fn issues_wrap_instead_of_truncating_at_80_and_120_columns() {
    for cols in [80usize, 120] {
        let lines = frame_with_issue(cols);
        let joined: String = lines.join("\n");
        assert!(joined.contains("[Extension issues]"), "{cols}: 缺少段头");
        // 折行后整段文本必须完整出现（去掉换行后仍是原文的字符序列）。
        assert!(
            joined.contains("不应被截断"),
            "{cols}: 折行不应截断尾部：{joined}"
        );
        // 每一行都不超过宽度。
        for line in &lines {
            assert!(
                pi_tui::display_width(line) <= cols,
                "{cols}: 行超宽：{line:?}"
            );
        }
    }
}
