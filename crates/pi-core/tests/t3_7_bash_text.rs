//! P3-9：bash 工具的纯逻辑——超时校验、尾部截断、退出码文案。
//!
//! 判据：`bash.ts:22-33、320-372`（见 `spec/e-tools.md` 第 5 节）。

use pi_core::tools::bash_text::{
    append_status, exit_status_message, format_output, resolve_timeout_ms, truncate_tail,
    TimeoutError, MAX_TIMEOUT_SECONDS,
};
use pi_core::tools::read_text::DEFAULT_MAX_BYTES;

#[test]
fn no_timeout_means_no_timeout() {
    assert_eq!(resolve_timeout_ms(None).expect("应通过"), None);
}

#[test]
fn timeout_seconds_become_milliseconds() {
    assert_eq!(resolve_timeout_ms(Some(1.0)).expect("应通过"), Some(1000));
    assert_eq!(resolve_timeout_ms(Some(2.5)).expect("应通过"), Some(2500));
}

#[test]
fn invalid_timeouts_use_the_typescript_messages() {
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = resolve_timeout_ms(Some(bad)).expect_err("应报错");
        assert_eq!(error, TimeoutError::NotFiniteOrNonPositive);
        assert_eq!(
            error.to_string(),
            "Invalid timeout: must be a finite number of seconds"
        );
    }
    let error = resolve_timeout_ms(Some(f64::from(u32::MAX))).expect_err("超上限应报错");
    assert_eq!(
        error.to_string(),
        format!("Invalid timeout: maximum is {MAX_TIMEOUT_SECONDS} seconds")
    );
}

#[test]
fn small_output_is_not_truncated() {
    let tail = truncate_tail("a\nb\nc", 10, DEFAULT_MAX_BYTES);
    assert!(!tail.truncated);
    assert_eq!(tail.output_lines, 3);
}

#[test]
fn tail_truncation_keeps_the_end() {
    let tail = truncate_tail("a\nb\nc\nd", 2, DEFAULT_MAX_BYTES);
    assert!(tail.truncated);
    assert_eq!(tail.content, "c\nd", "保留的是末尾而不是开头");
    assert_eq!(tail.total_lines, 4);
    assert_eq!(tail.output_lines, 2);
}

#[test]
fn format_output_reports_the_line_range_and_full_output_path() {
    let tail = truncate_tail("a\nb\nc\nd", 2, DEFAULT_MAX_BYTES);
    let text = format_output(
        &tail.content,
        &tail,
        Some("/tmp/pi-bash-x.log"),
        "(no output)",
    );
    assert_eq!(
        text,
        "c\nd\n\n[Showing lines 3-4 of 4. Full output: /tmp/pi-bash-x.log]"
    );
}

#[test]
fn format_output_uses_empty_text_when_nothing_captured() {
    let tail = truncate_tail("", 10, DEFAULT_MAX_BYTES);
    let text = format_output("", &tail, None, "(no output)");
    assert_eq!(text, "(no output)");
}

#[test]
fn byte_truncated_tail_reports_the_byte_limit() {
    // 每行 10 字节，max_bytes = 25 → 从尾部只放得下 2 行。
    let content = "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc";
    let tail = truncate_tail(content, 100, 25);
    assert!(tail.truncated);
    assert_eq!(tail.content, "bbbbbbbbbb\ncccccccccc");
    let text = format_output(&tail.content, &tail, Some("p"), "(no output)");
    // 注意：TS 这里写的是 `formatSize(DEFAULT_MAX_BYTES)`——**硬编码默认值 50KB**，
    // 而不是本次实际生效的上限。Rust 侧照抄了这个行为（与 TS 一致优先于"看起来更合理"）。
    assert!(
        text.contains("(50.0KB limit)"),
        "按字节截断时要标出默认上限：{text}"
    );
}

#[test]
fn append_status_separates_with_a_blank_line_only_when_there_is_output() {
    assert_eq!(
        append_status("out", "Command exited with code 1"),
        "out\n\nCommand exited with code 1"
    );
    assert_eq!(append_status("", "Command aborted"), "Command aborted");
}

#[test]
fn exit_status_messages_follow_the_typescript_text() {
    assert_eq!(exit_status_message(Some(0)), None, "正常退出没有状态文案");
    assert_eq!(
        exit_status_message(Some(1)),
        Some("Command exited with code 1".to_string())
    );
    assert_eq!(
        exit_status_message(None),
        Some("Command terminated without an exit code".to_string())
    );
}
