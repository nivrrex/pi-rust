//! `/settings` 的主题菜单必须列出**自定义主题**（不只内置两套）。
//!
//! 此前 `tui.rs` 里硬编码 `vec!["dark", "light"]` ⇒ `--theme <dir>` 载入的主题
//! 在界面上看不到，用户只能靠 `--use-theme` 手输名字。

use pi_tui::theme::{available_names, register_custom, Theme};

#[test]
fn available_names_include_registered_custom_themes() {
    let tmp = tempfile::tempdir().expect("tmp");
    std::fs::write(
        tmp.path().join("solar.json"),
        r##"{"name":"solar","colors":{"accent":"#ff8800"}}"##,
    )
    .expect("写主题");
    register_custom(Theme::from_dir(tmp.path()));

    let names = available_names();
    assert!(
        names.contains(&"dark".to_string()),
        "内置 dark 必在：{names:?}"
    );
    assert!(
        names.contains(&"light".to_string()),
        "内置 light 必在：{names:?}"
    );
    assert!(
        names.contains(&"solar".to_string()),
        "自定义主题应列出：{names:?}"
    );
}
