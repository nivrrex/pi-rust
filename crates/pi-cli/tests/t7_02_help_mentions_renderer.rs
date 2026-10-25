//! P7-5e / W3：`--help` 必须写明 `PI_TUI_RENDERER` 的取值、默认值与非法值行为。
//!
//! 反证：删掉帮助里的那三行 ⇒ 本测试转红（见 notes/progress.md 的红绿原文）。

use std::process::Command;

#[test]
fn help_documents_the_tui_renderer_switch() {
    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--help")
        .output()
        .expect("能启动 pi-rs --help");
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    for needle in [
        "PI_TUI_RENDERER",
        "r1=整屏重画（默认）",
        "r2=行级 diff",
        "回退 r1",
    ] {
        assert!(
            text.contains(needle),
            "--help 输出缺少 {needle:?}；实得：\n{text}"
        );
    }
}
