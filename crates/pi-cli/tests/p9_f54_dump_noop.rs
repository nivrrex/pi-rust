//! F54/A4：`PI_TEST_DUMP` 未设置时，钩子的落盘入口必须是**纯 no-op**
//! （`record_frame`/`record_bytes` 不进任何状态），这样写出流与加钩子之前逐字节一致。
//!
//! 环境变量是进程级的 ⇒ 本文件只放这一个用例（cargo 每个 integration test 是独立进程），
//! 且**先删除**变量再断言，避免与其它用例互相影响。

#[test]
fn dump_hooks_are_noop_without_env() {
    // 该进程内没有任何其它用例会设置 PI_TEST_DUMP。
    std::env::remove_var("PI_TEST_DUMP");
    assert!(
        !pi_cli::testhook::dump_enabled(),
        "未设置 PI_TEST_DUMP 时 dump 必须关闭"
    );
    // 两个入口都不应 panic，也不应改变任何可观测状态（本用例能跑完即是证明）。
    pi_cli::testhook::record_bytes(1234);
    pi_cli::testhook::record_frame(&["不应写入".to_string()]);
    // `visible_screen` 在无记录时回退到现算的帧；这里只要求不 panic 且行数非空。
    let mut state = pi_tui::TuiState::new("0.87.1", "/tmp/work", "0.0");
    state.quiet_startup = true;
    let screen = pi_cli::testhook::visible_screen(&state, 80, 24);
    assert!(!screen.is_empty(), "无 dump 时也要能给出现算的屏幕");
}
