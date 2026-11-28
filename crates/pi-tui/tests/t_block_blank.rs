//! F47/W1：块间空行必须由**源码里是否真有空行**决定（等价于 marked 的 `space` token）。
//!
//! ref `packages/tui/src/components/markdown.ts` 每个块的 `case` 都是
//! 「`nextTokenType && nextTokenType !== "space"` 才补空行」，`paragraph` 另加
//! `!== "list"`（`:487-497`）；`space` token 自己再输出一行（`:617-620`）。
//! 因此规则是：**相邻块之间源码有空行 ⇒ 恰好一行空行；无空行 ⇒ 只有 `paragraph`
//! 后面紧跟列表时是 0 行，其余仍是 1 行**（块自己补）。
use pi_tui::{render_markdown_styled_with_width, theme, theme::Palette};

fn lines(md: &str, width: usize) -> Vec<String> {
    let palette = Palette::of(&theme::Theme::builtin("dark"));
    let (out, _, _) = render_markdown_styled_with_width(md, &palette, Some(width));
    out.into_iter().map(|l| l.trim_end().to_string()).collect()
}

fn blanks_between_pairs(out: &[String]) -> usize {
    out.iter().filter(|l| l.trim().is_empty()).count()
}

/// 段落 → 列表：源码**无**空行时 TS 不补空行（ref `:495` 的 `!== "list"`）。
#[test]
fn paragraph_then_list_tight_has_no_blank() {
    for md in ["段（三份）：\n- 甲项一\n", "段（三份）：\n1. 甲项一\n"] {
        let out = lines(md, 110);
        assert_eq!(out.len(), 2, "应只有两行：{out:?}");
        assert!(!out[1].trim().is_empty(), "第二行应是列表项：{out:?}");
    }
}

/// 段落 → 列表：源码**有**空行 ⇒ 保留一行空行（那个空行是 `space` token）。
#[test]
fn paragraph_then_list_loose_keeps_one_blank() {
    for md in ["段（三份）：\n\n- 甲项一\n", "段（三份）：\n\n1. 甲项一\n"] {
        let out = lines(md, 110);
        assert_eq!(out.len(), 3, "应有三行（含一空行）：{out:?}");
        assert!(out[1].trim().is_empty(), "第二行应是空行：{out:?}");
    }
}

/// 其余块对：无论有无空行，块后都是**恰好一行**空行（块自己补 or `space` token 补）。
#[test]
fn other_block_pairs_always_exactly_one_blank() {
    let pairs = [
        (
            "段落→代码块",
            "段：\n```\nx\n```\n",
            "段：\n\n```\nx\n```\n",
        ),
        (
            "代码块→列表",
            "```\nx\n```\n- 甲\n",
            "```\nx\n```\n\n- 甲\n",
        ),
        ("标题→列表", "## 标题\n- 甲\n", "## 标题\n\n- 甲\n"),
        ("段落→引用", "段\n> 引\n", "段\n\n> 引\n"),
    ];
    for (name, tight, loose) in pairs {
        for md in [tight, loose] {
            let out = lines(md, 110);
            assert_eq!(
                blanks_between_pairs(&out),
                1,
                "{name}：应恰好一行空行，实际 {out:?}"
            );
        }
    }
}

/// 列表 → 段落在源码里**无空行**时是 CommonMark 的懒续行 ⇒ 续行不得再带列表标记
/// （ref `markdown.ts:758-781` 的首行前缀只给第一行）。
#[test]
fn list_lazy_continuation_has_no_marker() {
    let out = lines("- 丁项一\n这一小节说明：\n", 110);
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(out[1].starts_with("   "), "续行应缩进且无标记：{out:?}");
    assert!(!out[1].contains('-'), "续行不得带 `-` 标记：{out:?}");
}
