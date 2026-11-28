//! F53c：写出层不变量——整帧输出里**不得出现相邻 ESC**、也不得出现孤立 ESC。
//!
//! 背景：`write_spans` 曾把行尾 `SEGMENT_RESET`（`\x1b[0m\x1b]8;;\x07`）当普通字符
//! 逐字写出，与"样式切换时的复位"拼成 `ESC ESC[0m`；终端把裸 ESC 当可见字符，
//! 屏幕上出现 `PY0m0m` 并多占一行（真实会话实测，real-parity 18 格整体位移 1 行）。
use pi_tui::diff::{diff_render_with_cursor_styled, write_spans, DiffState};
use pi_tui::style::{LineStyle, Span};
use pi_tui::{render_all, TranscriptItem, TuiState};

fn state() -> TuiState {
    let mut st = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    st.transcript.push(TranscriptItem::ToolCall {
        id: "c1".into(),
        name: "read".into(),
        summary: "C:/x/收官攻坚、蓄势启航/阜阳分公司“收官攻坚、蓄势启航”专项行动推进情况汇报.md"
            .into(),
        args_json: None,
        tool_known: Some(true),
        output: None,
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        collapsed_row: None,
    });
    st
}

/// 断言一段终端输出里没有相邻 ESC、也没有孤立 ESC（后面不是 `[`/`]`）。
fn assert_clean(bytes: &str, where_: &str) {
    let b = bytes.as_bytes();
    for (i, by) in b.iter().enumerate() {
        if *by != 0x1b {
            continue;
        }
        let next = b.get(i + 1).copied();
        assert!(
            matches!(next, Some(b'[') | Some(b']')),
            "{where_}: 位置 {i} 出现孤立/相邻 ESC（下一个字节 {next:?}）；\
             上下文 {:?}",
            &bytes[i.saturating_sub(40)..(i + 40).min(bytes.len())]
        );
    }
}

/// `write_spans` 直接处理"带行尾控制序列"的行：不得产出相邻/孤立 ESC。
///
/// 这是**改前必失败**的最小用例：行尾带上 `SEGMENT_RESET`（`\x1b[0m\x1b]8;;\x07`），
/// spans 覆盖正文；修复前 `write_spans` 把该序列当普通字符逐字写出，
/// 与"样式切换时的复位"拼成 `ESC ESC[0m`。
#[test]
fn write_spans_ignores_trailing_control_sequence_for_styling() {
    let line = " .md\u{1b}[0m\u{1b}]8;;\u{7}";
    let spans = vec![
        Span::new(0, 1, LineStyle::default()),
        Span::new(1, 4, LineStyle::fg((31, 35, 40)).bold()),
    ];
    let mut out = String::new();
    write_spans(&mut out, line, &spans, 20);
    assert_clean(&out, "write_spans");
    // 行尾控制序列必须保留（OSC 8 要闭合）。
    assert!(
        out.contains("\u{1b}]8;;\u{7}"),
        "行尾控制序列被吞掉：{out:?}"
    );
}

/// 带长路径的工具标题行（行尾有 `SEGMENT_RESET`）：整帧渲染 + 差分写出后仍必须干净。
#[test]
fn frame_output_has_no_dangling_escape() {
    let st = state();
    let frame = render_all(&st, 110, 40);
    let mut diff = DiffState::new();
    let styles = vec![None; frame.len()];
    let spans = vec![Vec::new(); frame.len()];
    let (bytes, cursor) =
        diff_render_with_cursor_styled(&mut diff, &frame, &styles, &spans, 110, 40)
            .expect("diff_render 不应报错");
    assert_clean(&bytes, "首帧");
    assert_clean(&cursor, "首帧光标");
    // 再跑一帧（命中差分路径）。
    let (bytes2, _) = diff_render_with_cursor_styled(&mut diff, &frame, &styles, &spans, 110, 40)
        .expect("diff_render 不应报错");
    assert_clean(&bytes2, "第二帧");
}

/// 行尾控制序列本身要**原样保留**（不能被丢掉，否则 OSC 8 超链接不闭合）。
#[test]
fn trailing_segment_reset_is_preserved() {
    let st = state();
    let frame = render_all(&st, 110, 40);
    let mut diff = DiffState::new();
    let styles = vec![None; frame.len()];
    let spans = vec![Vec::new(); frame.len()];
    let (bytes, _) = diff_render_with_cursor_styled(&mut diff, &frame, &styles, &spans, 110, 40)
        .expect("diff_render 不应报错");
    let opens = bytes.matches("\u{1b}]8;;").count();
    assert!(opens > 0, "OSC 8 序列应仍在输出里");
}
