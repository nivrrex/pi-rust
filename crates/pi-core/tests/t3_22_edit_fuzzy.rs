//! P3.2 / Q15：edit 的模糊匹配规则（`edit-diff.ts:11-57、207-251`）。
//!
//! **注意**：NFKC 未实现（见 `normalize_for_fuzzy_match` 的文档），因此这里只覆盖
//! 源码里逐条列出的显式替换规则与换行处理，不覆盖全角/兼容字符。

use pi_core::tools::edit_text::{
    count_occurrences, detect_line_ending, fuzzy_find_text, normalize_for_fuzzy_match,
    normalize_to_lf, restore_line_endings,
};

#[test]
fn line_endings_are_normalized_and_restored() {
    assert_eq!(normalize_to_lf("a\r\nb\rc"), "a\nb\nc");
    assert_eq!(detect_line_ending("a\r\nb"), "\r\n");
    assert_eq!(detect_line_ending("a\nb\r\n"), "\n", "LF 在前就是 LF");
    assert_eq!(detect_line_ending("no newline"), "\n");
    assert_eq!(restore_line_endings("a\nb", "\r\n"), "a\r\nb");
    assert_eq!(restore_line_endings("a\nb", "\n"), "a\nb");
}

#[test]
fn trailing_whitespace_is_stripped_per_line() {
    assert_eq!(
        normalize_for_fuzzy_match("a  \nb\t\n  c"),
        "a\nb\n  c",
        "只去**行尾**空白，行首保留"
    );
}

#[test]
fn smart_quotes_dashes_and_special_spaces_are_folded() {
    // 每条规则都取自 `edit-diff.ts:34-57` 的显式列表。
    assert_eq!(normalize_for_fuzzy_match("it\u{2019}s"), "it's");
    assert_eq!(normalize_for_fuzzy_match("\u{201C}hi\u{201D}"), "\"hi\"");
    assert_eq!(normalize_for_fuzzy_match("a\u{2013}b\u{2014}c"), "a-b-c");
    assert_eq!(normalize_for_fuzzy_match("a\u{2212}b"), "a-b");
    assert_eq!(normalize_for_fuzzy_match("a\u{00A0}b"), "a b");
    assert_eq!(normalize_for_fuzzy_match("a\u{3000}b"), "a b");
    assert_eq!(normalize_for_fuzzy_match("a\u{2002}b"), "a b");
}

#[test]
fn exact_match_wins_and_does_not_normalize() {
    let content = "alpha\nbeta\n";
    let result = fuzzy_find_text(content, "alpha");
    assert!(result.found);
    assert!(!result.used_fuzzy);
    assert_eq!(result.index, 0);
    assert_eq!(result.match_length, "alpha".len());
    assert_eq!(
        result.content_for_replacement, content,
        "精确命中不改基准内容"
    );
}

#[test]
fn fuzzy_match_uses_normalized_space() {
    // 精确匹配必须失败才谈得上模糊：`oldText` 跨了两行，而原文第一行有尾随空格。
    let content = "alpha   \nbeta\n";
    let old = "alpha\nbeta";
    assert!(
        !content.contains(old),
        "精确匹配应当失败（这正是本用例的前提）"
    );
    let result = fuzzy_find_text(content, old);
    assert!(result.found);
    assert!(result.used_fuzzy, "行尾空白差异应走模糊匹配");
    assert_eq!(result.index, 0);
    assert_eq!(
        result.content_for_replacement, "alpha\nbeta\n",
        "模糊命中时基准内容是**归一化后**的"
    );
}

#[test]
fn fuzzy_match_fails_when_nothing_matches() {
    let result = fuzzy_find_text("abc", "zzz");
    assert!(!result.found);
    assert!(!result.used_fuzzy);
}

#[test]
fn occurrences_are_counted_in_normalized_space() {
    // 三行的行尾空白不同，但归一化后都是 `same`。
    let content = "same\nsame  \nsame\t\n";
    assert_eq!(count_occurrences(content, "same"), 3);
    assert_eq!(count_occurrences(content, "other"), 0);
    assert_eq!(count_occurrences(content, ""), 0, "空 oldText 不去数");
}

#[test]
fn nfkc_is_documented_as_unimplemented() {
    // 全角字符依赖 NFKC；当前**故意不处理**，这个断言把"未实现"钉住，
    // 免得以后有人以为已经支持了。
    let fullwidth = "\u{FF21}"; // Ａ
    assert_eq!(
        normalize_for_fuzzy_match(fullwidth),
        fullwidth,
        "NFKC 未实现：全角字符保持原样"
    );
}

#[test]
fn untouched_lines_keep_their_original_whitespace() {
    use pi_core::tools::edit_text::{apply_edits, Edit};
    // 第二行有尾随空格但**没被 edit 碰到**——它必须原样保留（`edit-diff.ts:137-165`
    // 的 `applyReplacementsPreservingUnchanged`）。`oldText` 跨两行且第一行有尾随空格，
    // 因此精确匹配会失败、走模糊路径。
    let content = "alpha   \nbeta\ngamma   \n";
    let edits = vec![Edit {
        old_text: "alpha\nbeta".to_string(),
        new_text: "ALPHA\nBETA".to_string(),
    }];
    let result = apply_edits(content, &edits, "f.txt").expect("应成功");
    assert_eq!(
        result, "ALPHA\nBETA\ngamma   \n",
        "未改动行（gamma   ）的尾随空格必须保留"
    );
}

#[test]
fn fuzzy_path_still_reports_duplicates_in_normalized_space() {
    use pi_core::tools::edit_text::{apply_edits, Edit};
    let content = "same\nsame  \n";
    let edits = vec![Edit {
        old_text: "same".to_string(),
        new_text: "SAME".to_string(),
    }];
    // "same" 精确命中第一行（"same" 是 "same" 的子串），所以这里其实走精确路径；
    // 但归一化空间里它出现两次，应当报不唯一。
    let error = apply_edits(content, &edits, "f.txt").expect_err("应报不唯一");
    assert!(
        matches!(
            error,
            pi_core::tools::edit_text::EditError::Duplicate { occurrences: 2, .. }
        ),
        "实际：{error}"
    );
}
