//! P9-F14：扩展加载问题必须在启动屏可见（ref `interactive-mode.ts:1874-1894` 的
//! `[Extension issues]` 段）。

use pi_tui::{apply, TuiEvent, TuiState};

#[test]
fn extension_issues_are_rendered_on_the_startup_screen() {
    let mut state = TuiState::new("0.87.1", "/tmp", "idle");
    apply(
        &mut state,
        TuiEvent::ExtensionIssues(vec!["未找到 jiti：.ts 扩展将跳过".to_string()]),
    );
    let lines = pi_tui::render_all(&state, 100, 30);
    let joined = lines.join("\n");
    assert!(
        joined.contains("[Extension issues]"),
        "应有问题段：{joined}"
    );
    assert!(joined.contains("未找到 jiti"), "应显示具体原因：{joined}");
}

#[test]
fn empty_issues_render_nothing() {
    let state = TuiState::new("0.87.1", "/tmp", "idle");
    let joined = pi_tui::render_all(&state, 100, 30).join("\n");
    assert!(
        !joined.contains("[Extension issues]"),
        "无问题时不该出现该段"
    );
}

/// F23/S2：启动屏紧凑列表的折行与 [Context] 路径显示。
#[test]
fn compact_list_wraps_like_ref_and_context_paths_are_shown_like_ref() {
    let mut state = TuiState::new("0.87.1", "/work", "idle");
    state.extension_labels = vec![
        "context-gateway".into(),
        "pi-agent-resilience".into(),
        "pi-ai-summary".into(),
        "pi-bash-guard".into(),
        "pi-handoff-gate".into(),
        "pi-interact".into(),
        "pi-remote-control".into(),
        "pi-secrets".into(),
        "web-read".into(),
        "web-search".into(),
    ];
    state.context_files = vec![
        "/work/AGENTS.md".into(),
        "/home/u/.pi/agent/AGENTS.md".into(),
    ];
    state.home = Some("/home/u".into());
    let lines = pi_tui::render_all(&state, 120, 30);
    let joined = lines.join("\n");
    assert!(
        joined.contains("  AGENTS.md"),
        "cwd 下的指令文件应显示相对路径：{joined}"
    );
    assert!(
        joined.contains("~/.pi/agent/AGENTS.md"),
        "家目录下的指令文件应缩写为 ~：{joined}"
    );
    // 列表在 120 列处折行，续行不加缩进（与 TS 真机一致）。
    let first = lines
        .iter()
        .find(|line| line.starts_with("  context-gateway"))
        .expect("应有 [Extensions] 列表行");
    assert!(
        pi_tui::display_width(first) <= 120,
        "折行后每行不得超过 120 列：{first}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("pi-secrets") || line.starts_with("  pi-secrets")),
        "续行应包含剩余名字：{joined}"
    );
}
