//! 运行期告警不得写 stderr（F238）。
//!
//! TUI 的 stderr 与画面共用同一个 PTY：在主循环画完一帧之后写 stderr，字节落在
//! **当前光标位置**——也就是编辑器/输入框那一行，于是"错误信息出现在用户输入框里"
//! （真机反馈）。首帧前写的会被后续帧覆盖，**会话中途**写的就留在屏幕上。
//!
//! 这里钉住三件事：
//! 1. TUI 模式下 `warn` 只入队、不写 stderr；
//! 2. `drain` 取走后队列为空，同一条消息不重复堆叠；
//! 3. 非 TUI 模式仍写 stderr（`-p`/rpc 下那是正当输出）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[test]
fn tui_mode_queues_instead_of_writing_stderr() {
    pi_cli::tui_notices::clear();
    pi_cli::tui_notices::set_tui_mode(true);
    pi_cli::tui_notices::warn("pi-rs: 写会话失败：disk full");
    pi_cli::tui_notices::warn("pi-rs: 写会话失败：disk full");
    let drained = pi_cli::tui_notices::drain();
    assert_eq!(
        drained,
        vec!["pi-rs: 写会话失败：disk full".to_string()],
        "同一条告警只保留一份（每帧重试也不刷屏）：{drained:?}"
    );
    assert!(pi_cli::tui_notices::drain().is_empty(), "取走后应为空");
    pi_cli::tui_notices::set_tui_mode(false);
}

#[test]
fn blank_messages_are_ignored() {
    pi_cli::tui_notices::clear();
    pi_cli::tui_notices::set_tui_mode(true);
    pi_cli::tui_notices::warn("   ");
    pi_cli::tui_notices::warn("");
    assert!(pi_cli::tui_notices::drain().is_empty(), "空告警不入队");
    pi_cli::tui_notices::set_tui_mode(false);
}

/// 非 TUI：走 stderr（不排队）。
#[test]
fn non_tui_mode_keeps_stderr() {
    pi_cli::tui_notices::clear();
    pi_cli::tui_notices::set_tui_mode(false);
    pi_cli::tui_notices::warn("pi-rs: 写会话失败：disk full");
    assert!(
        pi_cli::tui_notices::drain().is_empty(),
        "非 TUI 不应入队（应直接写 stderr）"
    );
}
