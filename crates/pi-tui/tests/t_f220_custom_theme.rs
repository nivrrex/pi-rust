//! 自定义主题：从 JSON 文件加载（ref `theme.ts:455-477` 的 `getCustomThemeInfos`）。
//!
//! 上游主题是 `{name, vars?, colors: {…}}` 的 JSON；pi-rs 的 `Theme::parse` 早已能解这个
//! 形状（内置 dark/light 就是从同样的 JSON 载入的），只缺一个"公开的、从文件/目录读"的入口。

use pi_tui::theme::Theme;

fn write_theme(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, body).expect("写主题");
    path
}

#[test]
fn theme_can_be_loaded_from_a_json_file() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = write_theme(
        tmp.path(),
        "solar",
        r##"{"name":"solar","colors":{"accent":"#ff8800","text":"#eeeeee"}}"##,
    );
    let theme = Theme::from_file(&path).expect("应能加载");
    assert_eq!(theme.name(), "solar");
    assert_eq!(theme.rgb("accent"), Some((0xff, 0x88, 0x00)));
    assert_eq!(theme.rgb("text"), Some((0xee, 0xee, 0xee)));
}

/// `vars` 间接引用也要能解析（与内置主题同一套解析）。
#[test]
fn theme_parses_vars_indirection() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = write_theme(
        tmp.path(),
        "v",
        r##"{"name":"v","vars":{"brand":"#123456"},"colors":{"accent":"brand"}}"##,
    );
    let theme = Theme::from_file(&path).expect("应能加载");
    assert_eq!(theme.rgb("accent"), Some((0x12, 0x34, 0x56)));
}

/// 非法主题（缺 `colors`）⇒ `Err`，调用方据此跳过并报告，而不是 panic。
#[test]
fn invalid_theme_is_an_error() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = write_theme(tmp.path(), "bad", r#"{"name":"bad"}"#);
    assert!(Theme::from_file(&path).is_err(), "缺 colors 应报错");
}

/// 目录扫描：`*.json` 全部尝试加载，非 json 忽略，坏文件跳过（ref 同样 catch 掉）。
#[test]
fn directory_scan_keeps_the_valid_ones() {
    let tmp = tempfile::tempdir().expect("tmp");
    write_theme(
        tmp.path(),
        "one",
        r##"{"name":"one","colors":{"accent":"#010101"}}"##,
    );
    write_theme(
        tmp.path(),
        "two",
        r##"{"name":"two","colors":{"accent":"#020202"}}"##,
    );
    write_theme(tmp.path(), "broken", "{not json");
    std::fs::write(tmp.path().join("readme.txt"), "ignore me").expect("写非 json");

    let themes = Theme::from_dir(tmp.path());
    let mut names: Vec<&str> = themes.iter().map(Theme::name).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["one", "two"], "应只收下有效的两个");
}

/// **缺色键的主题必须安全**：用户很可能只写几个颜色。
///
/// `Palette::of` 是两级回落（自定义 → 内置 dark → 硬编码默认），
/// 这里钉住它：只给 `accent` 时其余键从内置主题补上；**完全空的主题也不崩**。
#[test]
fn sparse_and_empty_themes_fall_back_to_builtin() {
    let sparse =
        Theme::from_json(r##"{"name":"min","colors":{"accent":"#ff8800"}}"##).expect("解析");
    let palette = pi_tui::theme::Palette::of(&sparse);
    assert_eq!(palette.accent, (0xff, 0x88, 0x00), "自定义键应生效");
    let dark = pi_tui::theme::Palette::of(&Theme::builtin("dark"));
    assert_eq!(palette.text, dark.text, "缺失的键应回落到内置 dark");

    let empty = Theme::from_json(r##"{"name":"empty","colors":{}}"##).expect("解析");
    let empty_palette = pi_tui::theme::Palette::of(&empty);
    assert_eq!(empty_palette.accent, dark.accent, "空主题应整体回落到内置");
    assert_eq!(empty_palette.text, dark.text);
}
