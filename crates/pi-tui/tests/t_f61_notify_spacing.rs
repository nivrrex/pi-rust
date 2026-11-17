//! F61/44：连续 `notify`（`Status`/`Warning`/`Error`）之间的空行数必须与 ref 一致。
//!
//! ref 的 `showStatus`/`showWarning`/`showError` 各自是
//! `chatContainer.addChild(new Spacer(1))` + `Text`（`interactive-mode.ts:3709-3712`、
//! `:4469-4473`）——每条**前导** 1 行空白、**无尾随**空白。相邻两条之间因此只有 1 行。
//!
//! 此前 pi-rs 让 `Status` 补尾随、`Notice` 补前导，相邻时叠成 2 行。
//! 实测（T88）：TS 非空行 `11,13,15,17`、我们曾是 `11,14,16,17`。
//! `compare-tui` 会折叠连续空行所以看不见，`compare-style` 逐单元格比对才暴露。

use pi_tui::{render_all, TuiEvent, TuiState};

/// 造一个状态：三条通知（info → warning → error）+ 一条用户消息。
fn state_with_notifications() -> TuiState {
    let mut state = TuiState::new("0.0.0", "/tmp/work", "0.0%/128k (auto)");
    state.quiet_startup = true;
    // 三条 notify：info → warning → error（走与扩展 `ctx.ui.notify` 相同的路径）。
    for (kind, message) in [
        (None, "plugin ready"),
        (Some("warning"), "plugin degraded"),
        (Some("error"), "plugin failed"),
    ] {
        let _ = pi_tui::apply(
            &mut state,
            TuiEvent::ExtensionNotice {
                message: message.to_string(),
                kind: kind.map(str::to_string),
            },
        );
    }
    state
}

/// 相邻两条通知之间**只有 1 行**空白（ref 的 `Spacer(1)` 语义）。
#[test]
fn consecutive_notifications_are_separated_by_one_blank_line() {
    let state = state_with_notifications();
    let lines = render_all(&state, 100, 30);
    let find = |needle: &str| {
        lines
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("应渲染出 {needle}：{lines:#?}"))
    };
    let info = find("plugin ready");
    let warning = find("Warning: plugin degraded");
    let error = find("Error: plugin failed");
    assert_eq!(
        warning - info,
        2,
        "info → warning 应相隔 2 行（1 行内容 + 1 行空白）。实际 info={info} warning={warning}\n{lines:#?}"
    );
    assert_eq!(
        error - warning,
        2,
        "warning → error 同样相隔 2 行。实际 warning={warning} error={error}\n{lines:#?}"
    );
}
