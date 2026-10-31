//! P7-1/P7-2 W1(c)：`--tui-mode` 的 rc 与 stderr **逐字**钉住（对拍 TS 实测）。
//!
//! TS 实测（W1(d)/W1(b)）：
//! - 非法值 `--tui-mode foo` ⇒ exitCode=1、stderr = `Error: Invalid TUI mode "foo". Valid values: regular, fullscreen\n`
//! - 缺值（`--tui-mode` 位于行尾）⇒ exitCode=1、stderr = `Error: --tui-mode requires regular or fullscreen\n`
//!
//! 出处：`ref/.../cli/args.ts:213-224`（两条 error 诊断）与 `main.ts:608-615`（`Error: ` 前缀 + exit 1）。

use std::process::Command;

fn run(args: &[&str]) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .args(args)
        .output()
        .expect("启动 pi-rs");
    let code = output.status.code().unwrap_or(-1);
    (code, String::from_utf8_lossy(&output.stderr).to_string())
}

#[test]
fn invalid_tui_mode_matches_the_measured_typescript_behaviour() {
    let (code, stderr) = run(&["--tui-mode", "foo", "-p", "hi"]);
    assert_eq!(code, 1, "rc 必须为 1：{stderr:?}");
    assert_eq!(
        stderr, "Error: Invalid TUI mode \"foo\". Valid values: regular, fullscreen\n",
        "非法值文案必须与 TS 实测逐字一致"
    );
}

#[test]
fn missing_tui_mode_value_matches_the_measured_typescript_behaviour() {
    let (code, stderr) = run(&["--print", "hi", "--tui-mode"]);
    assert_eq!(code, 1, "rc 必须为 1：{stderr:?}");
    assert_eq!(
        stderr, "Error: --tui-mode requires regular or fullscreen\n",
        "缺值文案必须与 TS 实测逐字一致"
    );
}
