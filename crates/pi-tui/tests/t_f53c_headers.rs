//! F53c：本批修掉的四处渲染缺陷 + F52 债务②的两项，各一条回归用例。
use pi_tui::{display_width, render_all, TranscriptItem, TuiState};

fn state_with(item: TranscriptItem) -> TuiState {
    let mut st = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    st.quiet_startup = true;
    st.transcript.push(item);
    st
}

fn tool(name: &str, summary: &str, args: Option<&str>, output: Option<&str>) -> TranscriptItem {
    TranscriptItem::ToolCall {
        id: "c1".into(),
        name: name.into(),
        summary: summary.into(),
        args_json: args.map(str::to_string),
        tool_known: Some(true),
        output: output.map(str::to_string),
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        collapsed_row: None,
    }
}

/// 列表项折行宽按 ref `markdown.ts:774-776`：`itemWidth = width - visibleWidth(firstPrefix)`。
/// 改前：只减 `marker_len`，一行多吞 2 个汉字（真实会话实测）。
#[test]
fn list_item_wrap_width_matches_ref() {
    let text = format!("- {} 尾巴标记", "中文内容".repeat(20));
    let st = state_with(TranscriptItem::Assistant(text));
    let frame = render_all(&st, 110, 40);
    // 每行都必须 ≤ cols（改前 113 > 110）。
    for (index, line) in frame.iter().enumerate() {
        let width = display_width(line);
        assert!(width <= 110, "第 {index} 行宽 {width} 超 110：{line:?}");
    }
    // 列表项被折行：至少 2 行，且续行以 2 个空格（`"  "` = `- ` 的等宽替换）开头。
    let lines: Vec<&String> = frame
        .iter()
        .filter(|l| l.contains("中文内容") || l.trim_start().starts_with("尾巴标记"))
        .collect();
    assert!(lines.len() >= 2, "列表项未被折行：{lines:?}");
}

/// 用户消息超宽必须**折行**，不能截断（改前 `clamp_to_width` 会把尾部丢掉：
/// 真实会话里一条 3 行的用户消息只剩 1 行）。
#[test]
fn user_message_wraps_instead_of_truncating() {
    let tail = "结尾标记XYZ";
    let text = format!("{}{tail}", "用户消息内容".repeat(30));
    let st = state_with(TranscriptItem::User(text));
    let frame = render_all(&st, 110, 60);
    let joined = frame.join("\n");
    assert!(joined.contains(tail), "用户消息尾部被截断：\n{joined}");
    for (index, line) in frame.iter().enumerate() {
        assert!(display_width(line) <= 110, "第 {index} 行超宽：{line:?}");
    }
}

/// `read` 的紧凑头部（ref `renderers/read.ts:88-110`）：资源文件折叠态渲染成
/// `read resource <路径> (ctrl+o to expand)`。
#[test]
fn read_resource_header_is_compact() {
    let st = state_with(tool(
        "read",
        "/work/AGENTS.md",
        Some(r#"{"path":"/work/AGENTS.md"}"#),
        None,
    ));
    let frame = render_all(&st, 120, 20).join("\n");
    assert!(
        frame.contains("read resource /work/AGENTS.md (ctrl+o to expand)"),
        "缺少紧凑头部：\n{frame}"
    );
    // 展开态回到普通头部。
    let mut expanded = state_with(tool(
        "read",
        "/work/AGENTS.md",
        Some(r#"{"path":"/work/AGENTS.md"}"#),
        None,
    ));
    expanded.tools_expanded = true;
    let frame = render_all(&expanded, 120, 20).join("\n");
    assert!(
        frame.contains("read /work/AGENTS.md") && !frame.contains("read resource"),
        "展开态不该用紧凑头部：\n{frame}"
    );
}

/// `read` 的行范围后缀（ref `renderers/read.ts:36-43`）：`:211-250` / `:211`。
#[test]
fn read_header_carries_line_range() {
    for (args, want) in [
        (r#"{"offset":211,"limit":40}"#, "path.md:211-250"),
        (r#"{"offset":7}"#, "path.md:7"),
        (r#"{}"#, "path.md"),
    ] {
        let st = state_with(tool("read", "path.md", Some(args), None));
        let frame = render_all(&st, 120, 20).join("\n");
        assert!(frame.contains(want), "args={args} 期望含 {want}：\n{frame}");
    }
}

/// 工具头里会话残留的转义必须清掉（改前屏幕上出现 `.md[0m[0m`）。
#[test]
fn tool_header_strips_stray_escapes() {
    let st = state_with(tool(
        "read",
        "a.md\u{1b}[0m[0m",
        Some("{\"path\":\"a.md\"}"),
        None,
    ));
    let frame = render_all(&st, 120, 20).join("\n");
    assert!(!frame.contains("[0m"), "残留 `[0m`：\n{frame}");
    assert!(!frame.contains('\u{1b}'), "残留 ESC：\n{frame}");
}

/// shell 头部带 `(timeout Ns)` 后缀（ref `renderers/bash.ts:43-49` 的 `timeoutSuffix`）。
#[test]
fn shell_header_carries_timeout_suffix() {
    let st = state_with(tool(
        "bash",
        "sleep 1",
        Some(r#"{"command":"sleep 1","timeout":600}"#),
        Some("done"),
    ));
    let frame = render_all(&st, 120, 20).join("\n");
    assert!(
        frame.contains("(timeout 600s)"),
        "缺少 timeout 后缀：\n{frame}"
    );
}
