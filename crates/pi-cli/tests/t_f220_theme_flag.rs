//! `--theme <dir>` / `--no-themes`（ref `cli/args.ts:187-189` 与 `:198-199`）。
//!
//! `--theme` 追加自定义主题目录、按上游主题 JSON 格式载入并注册；
//! `--no-themes` 不加载（只留内置 `dark`/`light`）。

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn theme_flag_collects_directories() {
    let parsed = pi_cli::args::parse(&owned(&[
        "--theme",
        "/a/themes",
        "--theme",
        "/b/themes",
        "-p",
        "hi",
    ]));
    assert_eq!(parsed.theme_dirs, vec!["/a/themes", "/b/themes"]);
    assert!(!parsed.no_themes);
}

#[test]
fn no_themes_flag_is_accepted() {
    let parsed = pi_cli::args::parse(&owned(&["--no-themes", "-p", "hi"]));
    assert!(parsed.no_themes);
}

/// 缺值报错（与 `--use-theme` 同一处理）。
#[test]
fn theme_flag_without_a_value_is_rejected() {
    let parsed = pi_cli::args::parse(&owned(&["--theme"]));
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.message.contains("--theme")),
        "缺值应记诊断：{:?}",
        parsed.diagnostics
    );
}

/// 注册后的自定义主题能被 `resolve` 找到（内置的仍可用）。
#[test]
fn registered_custom_theme_resolves() {
    let tmp = tempfile::tempdir().expect("tmp");
    std::fs::write(
        tmp.path().join("solar.json"),
        r##"{"name":"solar","colors":{"accent":"#ff8800"}}"##,
    )
    .expect("写主题");
    let themes = pi_tui::theme::Theme::from_dir(tmp.path());
    assert_eq!(themes.len(), 1);
    pi_tui::theme::register_custom(themes);

    let resolved = pi_tui::theme::resolve("solar");
    assert_eq!(resolved.name(), "solar");
    assert_eq!(resolved.rgb("accent"), Some((0xff, 0x88, 0x00)));
    // 内置的仍然可用。
    assert_eq!(pi_tui::theme::resolve("light").name(), "light");
}
