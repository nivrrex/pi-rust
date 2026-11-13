//! F58/H3：`\t` 展开的**属性式**用例——每条排版路径喂含制表符的合成输入，
//! 断言 80/120 列下每行 `display_width_ansi ≤ cols`。
//!
//! 依据：`unicode-width` 把 `\t` 算 **0 列**，终端按制表位展开（最多 8 列）。
//! ref 的排版出口统一做 `replace(/\t/g, "   ")`：
//! `packages/tui/src/components/text.ts:61`、`markdown.ts:298`、
//! `coding-agent/.../components/diff.ts:18`、`coding-agent/src/core/tools/render-utils.ts:32`。
//! pi-rs 侧收敛到 `pi_tui::expand_tabs_for_layout`。
//!
//! **编辑器粘贴归一化例外**：ref `editor.ts:1146` 是 **4 空格**（`normalizeText`），
//! 与排版口径不同，不在本用例的断言范围内（它不参与"行宽"计算，见该处注释）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_tui::{render_all, PanelPlacement, TranscriptItem, TuiState, WidgetEntry};

fn base() -> TuiState {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0");
    state.quiet_startup = true;
    state
}

/// 含制表符的合成输入（`\t` 与可见字符交错，覆盖"行首/行中/行尾制表符"）。
fn tabbed() -> String {
    format!("a\tb\tc\t{}\tend", "\t".repeat(30))
}

fn tool_call(name: &str) -> TranscriptItem {
    TranscriptItem::ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        summary: "p".to_string(),
        args_json: None,
        output: None,
        is_error: false,
        started_at: None,
        took: None,
        render_lines: None,
        result_render_lines: None,
        result_payload: None,
        tool_known: None,
        collapsed_row: None,
    }
}

/// 收集「排版路径 → 状态」的全部组合。
fn cases() -> Vec<(&'static str, TuiState)> {
    let text = tabbed();
    let mut out: Vec<(&'static str, TuiState)> = Vec::new();

    let mut s = base();
    s.transcript.push(TranscriptItem::User(text.clone()));
    out.push(("user", s));

    let mut s = base();
    s.transcript.push(TranscriptItem::Assistant(text.clone()));
    out.push(("assistant", s));

    // markdown 代码块（ref `markdown.ts:298` 展开后走 highlight/原样）
    let mut s = base();
    s.transcript
        .push(TranscriptItem::Assistant(format!("```\n{text}\n```")));
    out.push(("assistant-code", s));

    // markdown 列表 / 表格 / 引用
    let mut s = base();
    s.transcript
        .push(TranscriptItem::Assistant(format!("- {text}")));
    out.push(("assistant-list", s));
    let mut s = base();
    s.transcript.push(TranscriptItem::Assistant(format!(
        "| a | b |\n|---|---|\n| {text} | x |"
    )));
    out.push(("assistant-table", s));
    let mut s = base();
    s.transcript
        .push(TranscriptItem::Assistant(format!("> {text}")));
    out.push(("assistant-quote", s));

    // thinking（可见 / 隐藏标签）
    let mut s = base();
    s.transcript.push(TranscriptItem::Thinking {
        text: text.clone(),
        label: None,
    });
    out.push(("thinking", s));
    let mut s = base();
    s.hide_thinking_block = true;
    s.transcript.push(TranscriptItem::Thinking {
        text: "x".to_string(),
        label: Some(text.clone()),
    });
    out.push(("thinking-hidden", s));

    // Status / Notice / BranchSummary
    let mut s = base();
    s.transcript.push(TranscriptItem::Status(text.clone()));
    out.push(("status", s));
    let mut s = base();
    s.transcript.push(TranscriptItem::Notice(text.clone()));
    out.push(("notice", s));
    let mut s = base();
    s.transcript.push(TranscriptItem::BranchSummary {
        summary: text.clone(),
        expanded: true,
    });
    out.push(("branch-expanded", s));
    let mut s = base();
    s.transcript.push(TranscriptItem::BranchSummary {
        summary: text.clone(),
        expanded: false,
    });
    out.push(("branch-collapsed", s));

    // SkillInvocation
    let mut s = base();
    s.transcript.push(TranscriptItem::SkillInvocation {
        name: "n".to_string(),
        content: text.clone(),
    });
    out.push(("skill", s));

    // BashExecution（`!` 命令块）
    let mut s = base();
    s.transcript.push(TranscriptItem::BashExecution {
        command: text.clone(),
        output: text.clone(),
        exit_code: Some(1),
        cancelled: false,
        truncated: false,
        full_output_path: None,
        exclude_from_context: false,
        running: false,
    });
    out.push(("bash-exec", s));

    // 工具块：shell 头/正文
    let mut s = base();
    if let TranscriptItem::ToolCall {
        summary, output, ..
    } = tool_call("bash")
    {
        // 用带制表符的命令与输出
        let mut item = tool_call("bash");
        if let TranscriptItem::ToolCall {
            summary: ref mut sm,
            output: ref mut op,
            ..
        } = item
        {
            *sm = text.clone();
            *op = Some(text.clone());
        }
        let _ = (summary, output);
        s.transcript.push(item);
    }
    out.push(("tool-bash", s));

    // write 工具：正文来自调用参数
    let mut s = base();
    let mut item = tool_call("write");
    if let TranscriptItem::ToolCall {
        args_json, output, ..
    } = &mut item
    {
        *args_json = Some(serde_json::json!({ "path": "p", "content": text.clone() }).to_string());
        *output = Some("ok".to_string());
    }
    s.transcript.push(item);
    out.push(("tool-write", s));

    // edit 工具：diff（ref `diff.ts:18` 展开）
    let mut s = base();
    let mut item = tool_call("edit");
    if let TranscriptItem::ToolCall { result_payload, .. } = &mut item {
        *result_payload = Some(
            serde_json::json!({
                "content": [],
                "isError": false,
                "details": { "diff": format!("@@ -1 +1 @@\n-\t{text}\n+\t{text}\n") },
            })
            .to_string(),
        );
    }
    s.transcript.push(item);
    out.push(("tool-edit-diff", s));

    // 扩展渲染行 / 结果渲染行（`renderCall` / `renderResult`）
    let mut s = base();
    let mut item = tool_call("ext_x");
    if let TranscriptItem::ToolCall {
        render_lines,
        tool_known,
        ..
    } = &mut item
    {
        *render_lines = Some(vec![text.clone()]);
        *tool_known = Some(true);
    }
    s.transcript.push(item);
    out.push(("ext-render", s));

    let mut s = base();
    let mut item = tool_call("ext_x");
    if let TranscriptItem::ToolCall {
        result_render_lines,
        tool_known,
        ..
    } = &mut item
    {
        *result_render_lines = Some(vec![text.clone()]);
        *tool_known = Some(true);
    }
    s.transcript.push(item);
    out.push(("ext-result", s));

    // 折叠行（扩展原型补丁给的额外行）
    let mut s = base();
    let mut item = tool_call("ext_x");
    if let TranscriptItem::ToolCall {
        collapsed_row,
        tool_known,
        ..
    } = &mut item
    {
        *collapsed_row = Some(vec![text.clone()]);
        *tool_known = Some(true);
    }
    s.transcript.push(item);
    out.push(("collapsed-row", s));

    // 扩展 widget（字符串行 / 组件行两种）
    for from_lines in [true, false] {
        let mut s = base();
        s.extension_widgets.insert(
            "k".to_string(),
            WidgetEntry {
                lines: vec![text.clone()],
                placement: "belowEditor".to_string(),
                from_lines,
            },
        );
        out.push((
            if from_lines {
                "widget-lines"
            } else {
                "widget-component"
            },
            s,
        ));
    }

    // 页脚 / 头部覆盖
    let mut s = base();
    s.footer_override = Some(vec![text.clone()]);
    out.push(("footer-override", s));
    let mut s = base();
    s.header_override = Some(vec![text.clone()]);
    out.push(("header-override", s));

    // 面板 / 补全菜单 / 编辑器输入
    let mut s = base();
    s.panel = Some((PanelPlacement::BelowEditor, vec![text.clone()]));
    out.push(("panel", s));
    let mut s = base();
    s.input = text.clone();
    out.push(("input", s));

    // 页脚状态 / 扩展状态 / custom lines
    let mut s = base();
    s.status = text.clone();
    out.push(("footer-status", s));
    let mut s = base();
    s.extension_status.insert("k".to_string(), text.clone());
    out.push(("ext-status", s));
    let mut s = base();
    s.custom_lines = Some(vec![text.clone()]);
    out.push(("custom-lines", s));

    // 启动告警 / Model scope / [Context] / [Skills] / [Extensions] / issues
    let mut s = base();
    s.startup_warnings.push(text.clone());
    out.push(("startup-warning", s));
    let mut s = base();
    s.model_scope = vec![text.clone()];
    out.push(("model-scope", s));
    let mut s = base();
    s.context_files = vec![text.clone()];
    out.push(("context", s));
    let mut s = base();
    s.skill_labels = vec![text.clone()];
    out.push(("skills", s));
    let mut s = base();
    s.extension_labels = vec![text.clone()];
    out.push(("ext-labels", s));
    let mut s = base();
    s.extension_paths = vec![format!("/x/{text}")];
    s.extension_labels = vec!["e".to_string()];
    s.tools_expanded = true;
    out.push(("ext-paths", s));
    let mut s = base();
    s.extension_issues = vec![text.clone()];
    s.extension_labels = vec!["e".to_string()];
    out.push(("ext-issues", s));

    out
}

/// 每条路径在 80/120 列下都不得出现 `visibleWidth > cols` 的行。
#[test]
fn every_layout_path_expands_tabs() {
    let mut failures: Vec<String> = Vec::new();
    for (tag, state) in cases() {
        for cols in [80usize, 120] {
            let over: Vec<(usize, usize)> = render_all(&state, cols, 600)
                .iter()
                .enumerate()
                .map(|(index, line)| (index, pi_tui::display_width_ansi(line)))
                .filter(|(_, width)| *width > cols)
                .collect();
            if !over.is_empty() {
                failures.push(format!("cols={cols} {tag} {over:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "以下路径渲染超宽：{failures:?}");
}

/// 渲染结果里不得残留裸制表符（排版阶段必须已展开）。
#[test]
fn rendered_frames_contain_no_raw_tabs() {
    let mut failures: Vec<String> = Vec::new();
    for (tag, state) in cases() {
        for line in render_all(&state, 100, 600) {
            // 编辑器输入行的 `\t` 由编辑器自己处理（4 空格，见 ref `editor.ts:1146`），
            // 这里只查"排版后仍带裸 `\t`"的行——那说明某条路径漏了展开。
            if line.contains('\t') {
                failures.push(format!("{tag}: {line:?}"));
                break;
            }
        }
    }
    assert!(failures.is_empty(), "渲染行里残留制表符：{failures:?}");
}

/// `expand_tabs_for_layout` 的行为契约（3 空格，与 ref `text.ts:61` 一致）。
#[test]
fn expand_tabs_uses_three_spaces() {
    assert_eq!(pi_tui::expand_tabs_for_layout("a\tb"), "a   b");
    assert_eq!(pi_tui::expand_tabs_for_layout("\t"), "   ");
    assert_eq!(pi_tui::expand_tabs_for_layout("no-tabs"), "no-tabs");
    // 多行：每个 `\t` 都展开，不按列对齐（ref 是纯替换，不是制表位计算）。
    assert_eq!(pi_tui::expand_tabs_for_layout("a\tb\nc\td"), "a   b\nc   d");
}
