//! F52/C：native 工具行的三条不变量。
//!
//! ref `packages/tui/src/components/text.ts:60-68`：
//! `text.replace(/\t/g, "   ")` → `wrapTextWithAnsi(normalizedText, contentWidth)`，
//! 即**制表符换 3 空格、按宽度折行、绝不截断**。超宽行若被截断，用户会看到
//! `(timeout 600s)` 这类尾部丢失（真机 E1）；命令里含 `\n` 时若不过滤换行，
//! 渲染结果会带裸 LF，终端自行换行造成阶梯缩进（真机 E2）。
//!
//! 不变量：
//! ① 任何一行不含裸 LF/CR/TAB，也不含 SGR/OSC8 之外的控制序列；
//! ② 内容保全：把该工具行的所有行去 ANSI、去空白后连接 == 原文去空白后连接；
//! ③ 每行可见宽度 ≤ cols。
use pi_tui::theme::{Palette, Theme};
use pi_tui::{render_all, TranscriptItem, TuiState};

fn tool_state(output: Option<&str>, summary: &str, expanded: bool) -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.tools_expanded = expanded;
    state.transcript.push(TranscriptItem::ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        summary: summary.into(),
        args_json: None,
        tool_known: Some(true),
        output: output.map(str::to_string),
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        collapsed_row: None,
    });
    let _ = Palette::of(&Theme::builtin("dark"));
    state
}

/// 去掉 SGR/OSC8 之后返回 `(可见文本, 是否含非法控制字符)`。
fn strip(text: &str) -> (String, bool) {
    let mut out = String::new();
    let mut bad = false;
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
                continue;
            }
            if index < bytes.len() && bytes[index] == b']' {
                while index < bytes.len() && bytes[index] != 0x07 {
                    index += 1;
                }
                if index < bytes.len() {
                    index += 1;
                }
                continue;
            }
            bad = true; // ESC 后面不是 CSI/OSC ⇒ 非法
            index += 1;
            continue;
        }
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        if ch == '\n' || ch == '\r' || ch == '\t' || ch.is_control() {
            bad = true;
        }
        out.push(ch);
        index += ch.len_utf8();
    }
    (out, bad)
}

fn squeeze(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn check(name: &str, summary: &str, output: Option<&str>, cols: usize, expanded: bool) {
    let state = tool_state(output, summary, expanded);
    let lines = render_all(&state, cols, 40);
    let mut joined = String::new();
    for (index, line) in lines.iter().enumerate() {
        let (visible, bad) = strip(line);
        assert!(
            !bad,
            "{name}/{cols}/{expanded}: 第 {index} 行含裸控制字符：{line:?}"
        );
        assert!(
            pi_tui::display_width(&visible) <= cols,
            "{name}/{cols}/{expanded}: 第 {index} 行宽 {} 超过 {cols}：{line:?}",
            pi_tui::display_width(&visible)
        );
        joined.push_str(&visible);
    }
    // 折叠态按 ref 只显示**尾部 5 行 + `... (N earlier lines…)`**（`renderers/bash.ts:18,86-97`），
    // 内容本就不全 ⇒ 内容保全只在展开态断言。
    if !expanded {
        return;
    }
    let expected = format!("{summary}{}", output.unwrap_or(""));
    // 头部把 `name summary` 拼成 ` bash <summary>`，故期望里含 "bash"。
    assert!(
        squeeze(&joined).contains(&squeeze(&expected)),
        "{name}/{cols}/{expanded}: 内容不全（疑似截断）\n  期望包含：{:?}\n  实际：{:?}",
        squeeze(&expected),
        squeeze(&joined)
    );
}

const HEREDOC: &str = "cat <<'EOF' > /tmp/x\n第一行 CJK 内容\n\t缩进行 with tab\nEOF";
const LONG_PATH: &str = "cd /root/Desktop/一个非常长的没有空格的中文路径目录名字用来测试折行而不是截断/and-a-very-long-ascii-segment-without-any-space-at-all";
const WITH_TIMEOUT: &str = "sleep 600 # (timeout 600s)";

#[test]
fn heredoc_command_keeps_all_lines_and_no_raw_controls() {
    for cols in [80, 110] {
        for expanded in [false, true] {
            check("heredoc", HEREDOC, Some("done"), cols, expanded);
        }
    }
}

#[test]
fn long_path_wraps_instead_of_truncating() {
    for cols in [80, 110] {
        for expanded in [false, true] {
            check("long-path", LONG_PATH, Some("ok"), cols, expanded);
        }
    }
}

#[test]
fn timeout_suffix_survives_wrapping() {
    for cols in [80, 110] {
        for expanded in [false, true] {
            check("timeout", WITH_TIMEOUT, Some("exit 0"), cols, expanded);
        }
    }
}

#[test]
fn tool_body_with_tabs_and_long_output() {
    let body = (0..60)
        .map(|i| format!("\t行 {i} with\ttabs\tand CJK 内容填充填充填充填充填充填充"))
        .collect::<Vec<_>>()
        .join("\n");
    for cols in [80, 110] {
        for expanded in [false, true] {
            check("body", "run", Some(&body), cols, expanded);
        }
    }
}
