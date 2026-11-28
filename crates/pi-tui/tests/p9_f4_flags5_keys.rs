//! F4：push flags 从 7 改成 5（关掉位2=上报事件类型）后，**按键解析必须不变**。
//!
//! 位2 只影响"终端是否多发一次抬起事件"；位1（disambiguate）与位4（alternate keys）仍开着，
//! 所以 Shift+Enter、Ctrl+J、Home/End、功能键的 CSI-u 形态与 ref 一致。

use pi_tui::input::parse_key;

fn kitty(seq: &str) -> Option<String> {
    parse_key(seq, true)
}

#[test]
fn shift_enter_and_ctrl_j_are_unchanged() {
    // Shift+Enter：CSI-u 13;2（modifier 2 = shift）
    assert_eq!(kitty("\u{1b}[13;2u").as_deref(), Some("shift+enter"));
    // Ctrl+J = LF
    assert_eq!(kitty("\n").as_deref(), Some("shift+enter"));
    // Enter 本体
    assert_eq!(kitty("\r").as_deref(), Some("enter"));
}

#[test]
fn navigation_and_function_keys_are_unchanged() {
    assert_eq!(kitty("\u{1b}[H").as_deref(), Some("home"));
    assert_eq!(kitty("\u{1b}[F").as_deref(), Some("end"));
    // 注：带修饰的 Home/End（`\x1b[1;5H`）在 flags 5 与 7 下形态相同（修饰由位1/4 决定），
    // 与本批改动无关；这里只钉住无修饰形态，避免把既有解析差异混进本测试。
    assert_eq!(kitty("\u{1b}[5~").as_deref(), Some("pageUp"));
    assert_eq!(kitty("\u{1b}[6~").as_deref(), Some("pageDown"));
    assert_eq!(kitty("\u{1b}[3~").as_deref(), Some("delete"));
    assert_eq!(kitty("\u{1b}[Z").as_deref(), Some("shift+tab"));
    // 功能键：CSI 与 SS3 两种形态
    assert_eq!(kitty("\u{1b}[15~").as_deref(), Some("f5"));
    assert_eq!(kitty("\u{1b}OP").as_deref(), Some("f1"));
}

#[test]
fn presses_without_event_type_still_parse() {
    // flags=5 下终端不会追加 `:1`/`:2`/`:3`，但带后缀的形态仍必须能解析（终端可能自行上报）。
    assert_eq!(kitty("\u{1b}[97;5u").as_deref(), Some("ctrl+a"));
    assert_eq!(kitty("\u{1b}[97;5:1u").as_deref(), Some("ctrl+a"));
    assert_eq!(kitty("\u{1b}[97;5:2u").as_deref(), Some("ctrl+a"));
    // 抬起事件仍然被识别为 release 并被上层丢弃（`tui.ts:1073-1076` 语义）。
    assert!(pi_tui::input::is_release("\u{1b}[97;5:3u"));
}
