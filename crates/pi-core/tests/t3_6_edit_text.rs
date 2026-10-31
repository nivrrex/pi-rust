//! P3-8：edit 工具的纯逻辑——校验、替换、重叠与空改动。
//!
//! 判据：`edit-diff.ts:250-350`（见 `spec/e-tools.md` 第 4 节）。

use pi_core::tools::edit_text::{apply_edits, success_message, Edit, EditError};

fn edit(old: &str, new: &str) -> Edit {
    Edit {
        old_text: old.to_string(),
        new_text: new.to_string(),
    }
}

#[test]
fn single_edit_replaces_the_matched_text() {
    let result = apply_edits(
        "let a = 1;\nlet b = 2;\n",
        &[edit("let a = 1;", "let a = 42;")],
        "f.rs",
    )
    .expect("应成功");
    assert_eq!(result, "let a = 42;\nlet b = 2;\n");
}

#[test]
fn multiple_edits_all_target_the_original_content() {
    // 第二条针对**原文**匹配，不是增量匹配。
    let result = apply_edits(
        "aaa\nbbb\n",
        &[edit("aaa", "AAA"), edit("bbb", "BBB")],
        "f.txt",
    )
    .expect("应成功");
    assert_eq!(result, "AAA\nBBB\n");
}

#[test]
fn empty_edits_is_rejected() {
    let error = apply_edits("x", &[], "f").expect_err("应报错");
    assert_eq!(error, EditError::EmptyEdits);
    assert_eq!(
        error.to_string(),
        "Edit tool input is invalid. edits must contain at least one replacement."
    );
}

#[test]
fn empty_old_text_messages_differ_by_edit_count() {
    let single = apply_edits("x", &[edit("", "y")], "f.rs").expect_err("应报错");
    assert_eq!(single.to_string(), "oldText must not be empty in f.rs.");

    let multiple = apply_edits("x", &[edit("x", "y"), edit("", "z")], "f.rs").expect_err("应报错");
    assert_eq!(
        multiple.to_string(),
        "edits[1].oldText must not be empty in f.rs."
    );
}

#[test]
fn not_found_messages_differ_by_edit_count() {
    let single = apply_edits("abc", &[edit("zzz", "y")], "f.rs").expect_err("应报错");
    assert_eq!(
        single.to_string(),
        "Could not find the exact text in f.rs. The old text must match exactly including all whitespace and newlines."
    );

    let multiple =
        apply_edits("abc", &[edit("abc", "x"), edit("zzz", "y")], "f.rs").expect_err("应报错");
    assert_eq!(
        multiple.to_string(),
        "Could not find edits[1] in f.rs. The oldText must match exactly including all whitespace and newlines."
    );
}

#[test]
fn duplicate_match_is_rejected_with_the_occurrence_count() {
    let error = apply_edits("dup\ndup\n", &[edit("dup", "x")], "f.txt").expect_err("应报错");
    assert_eq!(
        error,
        EditError::Duplicate {
            path: "f.txt".to_string(),
            occurrences: 2
        }
    );
    assert!(error.to_string().contains("Found 2 occurrences"));
    assert!(error.to_string().contains("must be unique"));
}

#[test]
fn overlapping_edits_are_rejected() {
    let error = apply_edits("abcdefgh", &[edit("abcd", "x"), edit("cdef", "y")], "f.txt")
        .expect_err("应报错");
    match error {
        EditError::Overlap {
            path,
            previous,
            current,
        } => {
            assert_eq!(path, "f.txt");
            assert_eq!((previous, current), (0, 1));
        }
        other => panic!("应为 Overlap，实际 {other}"),
    }
}

#[test]
fn adjacent_edits_are_not_overlapping() {
    // 首尾相接（不重叠）应当允许。
    let result = apply_edits("abcd", &[edit("ab", "X"), edit("cd", "Y")], "f").expect("应成功");
    assert_eq!(result, "XY");
}

#[test]
fn no_change_error_differs_by_edit_count() {
    let single = apply_edits("abc", &[edit("abc", "abc")], "f.rs").expect_err("应报错");
    assert!(
        single.to_string().starts_with("No changes made to f.rs."),
        "实际：{single}"
    );
    assert!(single.to_string().contains("identical content"));

    // 两条**互不重叠**的 no-op 替换：重叠检查先于空改动检查，所以这里必须避开重叠
    // （两条都指向同一区间会先报 Overlap——TS 的判定顺序也是如此）。
    let multiple =
        apply_edits("ab\ncd", &[edit("ab", "ab"), edit("cd", "cd")], "f.rs").expect_err("应报错");
    assert_eq!(
        multiple.to_string(),
        "No changes made to f.rs. The replacements produced identical content."
    );
}

#[test]
fn success_message_matches_typescript() {
    assert_eq!(
        success_message("src/main.rs", 1),
        "Successfully replaced 1 block(s) in src/main.rs."
    );
}

#[test]
fn edits_are_applied_back_to_front_preserving_offsets() {
    // 第一条替换比原文长，若不从后往前处理会把第二条的偏移算错。
    let result = apply_edits(
        "one\ntwo\n",
        &[edit("one", "one-much-longer"), edit("two", "TWO")],
        "f",
    )
    .expect("应成功");
    assert_eq!(result, "one-much-longer\nTWO\n");
}
