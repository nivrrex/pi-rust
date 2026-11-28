//! P3-7：read 工具的纯逻辑——切片、截断与提示文案。
//!
//! 判据：`read.ts:140-183`、`truncate.ts:70-150` 的行为（见 `spec/e-tools.md` 第 2 节）。

use pi_core::tools::read_text::{
    render_text_read, truncate_head, ReadTextError, TruncatedBy, DEFAULT_MAX_BYTES,
    DEFAULT_MAX_LINES,
};

fn render(content: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    render_text_read(content, offset, limit, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES)
        .expect("不应报错")
        .0
}

#[test]
fn plain_read_returns_the_whole_file() {
    // **尾随换行原样保留**：golden/S02 里 TS 读 `alpha\nbeta\n` 输出的就是带尾换行的。
    // 我最初这里写的是 `"a\nb\nc"`（少了尾换行）——那是把当时的实现行为当成了期望，
    // 由 `t3_26_loop_scenarios` 的场景重放抓出。
    assert_eq!(render("a\nb\nc\n", None, None), "a\nb\nc\n");
    assert_eq!(render("a\nb\nc", None, None), "a\nb\nc");
}

#[test]
fn offset_is_one_based() {
    assert_eq!(render("a\nb\nc", Some(2), None), "b\nc");
    assert_eq!(render("a\nb\nc", Some(1), None), "a\nb\nc");
}

#[test]
fn offset_beyond_end_reports_the_typescript_message() {
    let error = render_text_read(
        "a\nb\nc",
        Some(10),
        None,
        DEFAULT_MAX_LINES,
        DEFAULT_MAX_BYTES,
    )
    .expect_err("越界应报错");
    assert_eq!(
        error,
        ReadTextError::OffsetBeyondEnd {
            offset: 10,
            total_lines: 3
        }
    );
    assert_eq!(
        error.to_string(),
        "Offset 10 is beyond end of file (3 lines total)"
    );
}

#[test]
fn limit_stops_early_and_hints_the_remainder() {
    // 用户给了 limit 但文件还有内容 → 追加 remaining 提示（read.ts:174-179）。
    let text = render("a\nb\nc\nd", Some(1), Some(2));
    assert_eq!(
        text,
        "a\nb\n\n[2 more lines in file. Use offset=3 to continue.]"
    );
}

#[test]
fn limit_covering_the_rest_has_no_hint() {
    assert_eq!(render("a\nb\nc", Some(1), Some(3)), "a\nb\nc");
}

#[test]
fn truncation_by_lines_reports_the_range() {
    // max_lines = 2 时按行截断。
    let text = render_text_read("a\nb\nc\nd", None, None, 2, DEFAULT_MAX_BYTES)
        .expect("不应报错")
        .0;
    assert_eq!(
        text,
        "a\nb\n\n[Showing lines 1-2 of 4. Use offset=3 to continue.]"
    );
}

#[test]
fn truncation_by_bytes_reports_the_limit() {
    // 每行 10 字节，max_bytes = 25 → 只放得下 2 行（10 + 1 + 10 = 21 ≤ 25，第 3 行会到 32）。
    let content = "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc";
    let text = render_text_read(content, None, None, DEFAULT_MAX_LINES, 25)
        .expect("不应报错")
        .0;
    assert_eq!(
        text,
        // 括号里是 `formatSize(DEFAULT_MAX_BYTES)` = 50.0KB（TS 的一位小数格式，由
        // `tools/oracle/truncate-oracle.mjs` 实测）；此前我按取整写成 0KB，是错的。
        format!("aaaaaaaaaa\nbbbbbbbbbb\n\n[Showing lines 1-2 of 3 ({} limit). Use offset=3 to continue.]", pi_core::tools::bash_text::format_size(50 * 1024))
    );
}

#[test]
fn first_line_exceeding_the_byte_limit_returns_no_content() {
    // 首行单独就超限 → 不给内容，指引用 bash（read.ts:159-166）。
    let content = "x".repeat(100);
    let text = render_text_read(&content, None, None, DEFAULT_MAX_LINES, 50)
        .expect("不应报错")
        .0;
    assert!(
        text.starts_with("[Line 1 is 100B, exceeds 50B limit. Use bash:"),
        "实际：{text}"
    );
    assert!(!text.contains("xxxx"), "首行超限时不应返回内容：{text}");
}

#[test]
fn truncate_head_never_returns_partial_lines() {
    let truncation = truncate_head("aaaaaaaaaa\nbbbbbbbbbb", DEFAULT_MAX_LINES, 15);
    assert!(truncation.truncated);
    assert_eq!(truncation.truncated_by, Some(TruncatedBy::Bytes));
    assert_eq!(truncation.content, "aaaaaaaaaa", "只保留能整行放下的部分");
    assert!(!truncation.first_line_exceeds_limit);
}

#[test]
fn empty_file_reads_as_empty() {
    assert_eq!(render("", None, None), "");
}

#[test]
fn truncation_boundaries_are_inclusive() {
    // 恰好等于行数上限时不算截断。
    let content = "a\nb";
    let truncation = truncate_head(content, 2, DEFAULT_MAX_BYTES);
    assert!(!truncation.truncated);
    assert_eq!(truncation.output_lines, 2);

    // 超过一行就截断。
    let truncation = truncate_head("a\nb", 1, DEFAULT_MAX_BYTES);
    assert!(truncation.truncated);
    assert_eq!(truncation.truncated_by, Some(TruncatedBy::Lines));
    assert_eq!(truncation.content, "a");
}
